use crate::daemon::log_writer::{self, OutputLine};
use crate::paths;
use crate::protocol::{ProcessInfo, ProcessState, Response, Stream as ProtoStream};
use crate::session::IdCounter;
use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use tokio::sync::broadcast;

const DEFAULT_MAX_LOG_BYTES: u64 = 50 * 1024 * 1024; // 50MB

pub struct ManagedProcess {
    pub name: String,
    pub id: String,
    pub command: String,
    pub child: Option<Child>,
    pub pid: u32,
    pub started_at: Instant,
    pub exit_code: Option<i32>,
}

pub struct ProcessManager {
    processes: HashMap<String, ManagedProcess>,
    id_counter: IdCounter,
    session: String,
    pub output_tx: broadcast::Sender<OutputLine>,
}

impl ProcessManager {
    pub fn new(session: &str) -> Self {
        let (output_tx, _) = broadcast::channel(1024);
        Self {
            processes: HashMap::new(),
            id_counter: IdCounter::new(),
            session: session.to_string(),
            output_tx,
        }
    }

    pub async fn spawn_process(&mut self, command: &str, name: Option<String>, cwd: Option<&str>) -> Response {
        let id = self.id_counter.next();
        let name = name.unwrap_or_else(|| id.clone());

        if self.processes.contains_key(&name) {
            return Response::Error { code: 1, message: format!("process already exists: {}", name) };
        }

        let log_dir = paths::log_dir(&self.session);
        let _ = std::fs::create_dir_all(&log_dir);

        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let mut child = match cmd.spawn()
        {
            Ok(c) => c,
            Err(e) => return Response::Error { code: 1, message: format!("failed to spawn: {}", e) },
        };

        let pid = child.id().unwrap_or(0);

        // Spawn output capture tasks via log_writer
        if let Some(stdout) = child.stdout.take() {
            let tx = self.output_tx.clone();
            let pname = name.clone();
            let path = log_dir.join(format!("{}.stdout", name));
            tokio::spawn(async move {
                log_writer::capture_output(stdout, &path, &pname, ProtoStream::Stdout, tx, DEFAULT_MAX_LOG_BYTES).await;
            });
        }
        if let Some(stderr) = child.stderr.take() {
            let tx = self.output_tx.clone();
            let pname = name.clone();
            let path = log_dir.join(format!("{}.stderr", name));
            tokio::spawn(async move {
                log_writer::capture_output(stderr, &path, &pname, ProtoStream::Stderr, tx, DEFAULT_MAX_LOG_BYTES).await;
            });
        }

        self.processes.insert(name.clone(), ManagedProcess {
            name: name.clone(), id: id.clone(), command: command.to_string(),
            child: Some(child), pid, started_at: Instant::now(), exit_code: None,
        });

        Response::RunOk { name, id, pid }
    }

    pub async fn stop_process(&mut self, target: &str) -> Response {
        let proc = match self.find_mut(target) {
            Some(p) => p,
            None => return Response::Error { code: 2, message: format!("process not found: {}", target) },
        };

        if let Some(ref child) = proc.child {
            let raw_pid = child.id().unwrap_or(0) as i32;
            if raw_pid > 0 {
                // Send SIGTERM first
                let pid = nix::unistd::Pid::from_raw(raw_pid);
                let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
            }
        }

        // Wait up to 10s for graceful exit, then SIGKILL
        if let Some(ref mut child) = proc.child {
            let wait_result = tokio::time::timeout(
                Duration::from_secs(10),
                child.wait()
            ).await;

            match wait_result {
                Ok(Ok(status)) => {
                    proc.exit_code = status.code();
                }
                _ => {
                    // Timed out or error — force kill
                    let _ = child.kill().await;
                    let status = child.wait().await;
                    proc.exit_code = status.ok().and_then(|s| s.code());
                }
            }
            proc.child = None;
        }

        Response::Ok { message: format!("stopped {}", target) }
    }

    pub async fn stop_all(&mut self) -> Response {
        let names: Vec<String> = self.processes.keys().cloned().collect();
        for name in names {
            self.stop_process(&name).await;
        }
        Response::Ok { message: "all processes stopped".into() }
    }

    pub async fn restart_process(&mut self, target: &str) -> Response {
        let (command, name) = match self.find(target) {
            Some(p) => (p.command.clone(), p.name.clone()),
            None => return Response::Error { code: 2, message: format!("process not found: {}", target) },
        };
        self.stop_process(target).await;
        self.processes.remove(&name);
        self.spawn_process(&command, Some(name), None).await
    }

    pub fn status(&mut self) -> Response {
        self.refresh_exit_states();
        let mut infos: Vec<ProcessInfo> = self.processes.values()
            .map(|p| ProcessInfo {
                name: p.name.clone(), id: p.id.clone(), pid: p.pid,
                state: if p.child.is_some() { ProcessState::Running } else { ProcessState::Exited },
                exit_code: p.exit_code,
                uptime_secs: if p.child.is_some() { Some(p.started_at.elapsed().as_secs()) } else { None },
                command: p.command.clone(),
            })
            .collect();
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        Response::Status { processes: infos }
    }

    pub fn is_process_exited(&mut self, target: &str) -> Option<Option<i32>> {
        self.refresh_exit_states();
        self.find(target).map(|p| if p.child.is_none() { p.exit_code } else { None })
    }

    fn refresh_exit_states(&mut self) {
        for proc in self.processes.values_mut() {
            if proc.child.is_some() && proc.exit_code.is_none() {
                if let Some(ref mut child) = proc.child {
                    if let Ok(Some(status)) = child.try_wait() {
                        proc.exit_code = status.code();
                        proc.child = None;
                    }
                }
            }
        }
    }

    pub fn has_process(&self, target: &str) -> bool {
        self.find(target).is_some()
    }

    fn find(&self, target: &str) -> Option<&ManagedProcess> {
        self.processes.get(target)
            .or_else(|| self.processes.values().find(|p| p.id == target))
    }

    fn find_mut(&mut self, target: &str) -> Option<&mut ManagedProcess> {
        if self.processes.contains_key(target) {
            self.processes.get_mut(target)
        } else {
            self.processes.values_mut().find(|p| p.id == target)
        }
    }
}
