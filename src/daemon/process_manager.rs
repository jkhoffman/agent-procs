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
const AUTO_PORT_MIN: u16 = 4000;
const AUTO_PORT_MAX: u16 = 4999;

/// Returns true if `name` is a valid DNS label: 1-63 lowercase alphanumeric/hyphen
/// chars, not starting or ending with a hyphen.
pub fn is_valid_dns_label(name: &str) -> bool {
    if name.is_empty() || name.len() > 63 {
        return false;
    }
    if name.starts_with('-') || name.ends_with('-') {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub struct ManagedProcess {
    pub name: String,
    pub id: String,
    pub command: String,
    pub cwd: Option<String>,
    pub env: HashMap<String, String>,
    pub child: Option<Child>,
    pub pid: u32,
    pub started_at: Instant,
    pub exit_code: Option<i32>,
    pub port: Option<u16>,
}

pub struct ProcessManager {
    processes: HashMap<String, ManagedProcess>,
    id_counter: IdCounter,
    session: String,
    pub output_tx: broadcast::Sender<OutputLine>,
    proxy_enabled: bool,
    next_auto_port: u16,
}

impl ProcessManager {
    pub fn new(session: &str) -> Self {
        let (output_tx, _) = broadcast::channel(1024);
        Self {
            processes: HashMap::new(),
            id_counter: IdCounter::new(),
            session: session.to_string(),
            output_tx,
            proxy_enabled: false,
            next_auto_port: AUTO_PORT_MIN,
        }
    }

    fn auto_assign_port(&mut self) -> Result<u16, String> {
        let start = self.next_auto_port;
        let assigned: std::collections::HashSet<u16> =
            self.processes.values().filter_map(|p| p.port).collect();
        let range_size = (AUTO_PORT_MAX - AUTO_PORT_MIN + 1) as usize;

        for i in 0..range_size {
            let candidate = AUTO_PORT_MIN
                + (((self.next_auto_port - AUTO_PORT_MIN) as usize + i) % range_size) as u16;
            if assigned.contains(&candidate) {
                continue;
            }
            // Bind-test: if we can bind, the port is free (listener drops immediately)
            if std::net::TcpListener::bind(("127.0.0.1", candidate)).is_ok() {
                self.next_auto_port = if candidate >= AUTO_PORT_MAX {
                    AUTO_PORT_MIN
                } else {
                    candidate + 1
                };
                return Ok(candidate);
            }
        }
        Err(format!(
            "no free port available in range {}-{} (started at {})",
            AUTO_PORT_MIN, AUTO_PORT_MAX, start
        ))
    }

    pub async fn spawn_process(
        &mut self,
        command: &str,
        name: Option<String>,
        cwd: Option<&str>,
        env: Option<&HashMap<String, String>>,
        port: Option<u16>,
    ) -> Response {
        let id = self.id_counter.next_id();
        let name = name.unwrap_or_else(|| id.clone());

        // Reject names that could cause path traversal in log files
        if name.contains('/') || name.contains('\\') || name.contains("..") || name.contains('\0') {
            return Response::Error {
                code: 1,
                message: format!("invalid process name: {}", name),
            };
        }

        // When proxy is active, names must be valid DNS labels for subdomain routing
        if self.proxy_enabled && !is_valid_dns_label(&name) {
            return Response::Error {
                code: 1,
                message: format!(
                    "invalid process name for proxy: '{}' (must be lowercase alphanumeric/hyphens, max 63 chars)",
                    name
                ),
            };
        }

        // Resolve the port: use explicit port, auto-assign if proxy is enabled, or None
        let resolved_port = if let Some(p) = port {
            Some(p)
        } else if self.proxy_enabled {
            match self.auto_assign_port() {
                Ok(p) => Some(p),
                Err(e) => return Response::Error { code: 1, message: e },
            }
        } else {
            None
        };

        if self.processes.contains_key(&name) {
            return Response::Error {
                code: 1,
                message: format!("process already exists: {}", name),
            };
        }

        let log_dir = paths::log_dir(&self.session);
        let _ = std::fs::create_dir_all(&log_dir);

        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        if let Some(p) = resolved_port {
            // Inject PORT and HOST; user-supplied env takes precedence
            let mut merged_env: HashMap<String, String> = HashMap::new();
            merged_env.insert("PORT".to_string(), p.to_string());
            merged_env.insert("HOST".to_string(), "127.0.0.1".to_string());
            if let Some(env_vars) = env {
                for (k, v) in env_vars {
                    merged_env.insert(k.clone(), v.clone());
                }
            }
            cmd.envs(&merged_env);
        } else if let Some(env_vars) = env {
            cmd.envs(env_vars);
        }
        // Put child in its own process group so we can signal the entire tree
        unsafe {
            cmd.pre_exec(|| {
                nix::unistd::setpgid(nix::unistd::Pid::from_raw(0), nix::unistd::Pid::from_raw(0))
                    .map_err(std::io::Error::other)?;
                Ok(())
            });
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return Response::Error {
                    code: 1,
                    message: format!("failed to spawn: {}", e),
                }
            }
        };

        let pid = child.id().unwrap_or(0);

        // Spawn output capture tasks via log_writer
        if let Some(stdout) = child.stdout.take() {
            let tx = self.output_tx.clone();
            let pname = name.clone();
            let path = log_dir.join(format!("{}.stdout", name));
            tokio::spawn(async move {
                log_writer::capture_output(
                    stdout,
                    &path,
                    &pname,
                    ProtoStream::Stdout,
                    tx,
                    DEFAULT_MAX_LOG_BYTES,
                )
                .await;
            });
        }
        if let Some(stderr) = child.stderr.take() {
            let tx = self.output_tx.clone();
            let pname = name.clone();
            let path = log_dir.join(format!("{}.stderr", name));
            tokio::spawn(async move {
                log_writer::capture_output(
                    stderr,
                    &path,
                    &pname,
                    ProtoStream::Stderr,
                    tx,
                    DEFAULT_MAX_LOG_BYTES,
                )
                .await;
            });
        }

        self.processes.insert(
            name.clone(),
            ManagedProcess {
                name: name.clone(),
                id: id.clone(),
                command: command.to_string(),
                cwd: cwd.map(|s| s.to_string()),
                env: env.cloned().unwrap_or_default(),
                child: Some(child),
                pid,
                started_at: Instant::now(),
                exit_code: None,
                port: resolved_port,
            },
        );

        let url = resolved_port.map(|p| format!("http://127.0.0.1:{}", p));
        Response::RunOk { name, id, pid, port: resolved_port, url }
    }

    pub async fn stop_process(&mut self, target: &str) -> Response {
        let proc = match self.find_mut(target) {
            Some(p) => p,
            None => {
                return Response::Error {
                    code: 2,
                    message: format!("process not found: {}", target),
                }
            }
        };

        if let Some(ref child) = proc.child {
            let raw_pid = child.id().unwrap_or(0) as i32;
            if raw_pid > 0 {
                // Signal the entire process group (child PID == PGID due to setpgid in pre_exec)
                let pgid = nix::unistd::Pid::from_raw(raw_pid);
                let _ = nix::sys::signal::killpg(pgid, nix::sys::signal::Signal::SIGTERM);
            }
        }

        // Wait up to 10s for graceful exit, then SIGKILL
        if let Some(ref mut child) = proc.child {
            let wait_result = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;

            match wait_result {
                Ok(Ok(status)) => {
                    proc.exit_code = status.code();
                }
                _ => {
                    // Timed out or error — force kill the process group
                    let raw_pid = proc.pid as i32;
                    if raw_pid > 0 {
                        let pgid = nix::unistd::Pid::from_raw(raw_pid);
                        let _ = nix::sys::signal::killpg(pgid, nix::sys::signal::Signal::SIGKILL);
                    }
                    let _ = child.wait().await;
                    proc.exit_code = Some(-9);
                }
            }
            proc.child = None;
        }

        Response::Ok {
            message: format!("stopped {}", target),
        }
    }

    pub async fn stop_all(&mut self) -> Response {
        let names: Vec<String> = self.processes.keys().cloned().collect();
        for name in names {
            self.stop_process(&name).await;
        }
        self.processes.clear();
        Response::Ok {
            message: "all processes stopped".into(),
        }
    }

    pub async fn restart_process(&mut self, target: &str) -> Response {
        let (command, name, cwd, env, port) = match self.find(target) {
            Some(p) => (
                p.command.clone(),
                p.name.clone(),
                p.cwd.clone(),
                p.env.clone(),
                p.port,
            ),
            None => {
                return Response::Error {
                    code: 2,
                    message: format!("process not found: {}", target),
                }
            }
        };
        self.stop_process(target).await;
        self.processes.remove(&name);
        let env = if env.is_empty() { None } else { Some(env) };
        self.spawn_process(&command, Some(name), cwd.as_deref(), env.as_ref(), port)
            .await
    }

    pub fn enable_proxy(&mut self) {
        self.proxy_enabled = true;
    }

    pub fn status(&mut self) -> Response {
        self.refresh_exit_states();
        Response::Status {
            processes: self.build_process_infos(),
        }
    }

    /// Returns `None` if process not found or still running.
    /// Returns `Some(exit_code)` if process has exited (exit_code is None for signal kills).
    pub fn is_process_exited(&mut self, target: &str) -> Option<Option<i32>> {
        self.refresh_exit_states();
        self.find(target).and_then(|p| {
            if p.child.is_none() {
                Some(p.exit_code)
            } else {
                None
            }
        })
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

    pub fn get_process_port(&self, name: &str) -> Option<u16> {
        self.processes
            .get(name)
            .and_then(|p| if p.child.is_some() { p.port } else { None })
    }

    /// Non-mutating status snapshot for use by the proxy status page.
    /// May show stale exit states since it skips `refresh_exit_states()`.
    pub fn status_snapshot(&self) -> Response {
        Response::Status {
            processes: self.build_process_infos(),
        }
    }

    fn build_process_infos(&self) -> Vec<ProcessInfo> {
        let mut infos: Vec<ProcessInfo> = self
            .processes
            .values()
            .map(|p| ProcessInfo {
                name: p.name.clone(),
                id: p.id.clone(),
                pid: p.pid,
                state: if p.child.is_some() {
                    ProcessState::Running
                } else {
                    ProcessState::Exited
                },
                exit_code: p.exit_code,
                uptime_secs: if p.child.is_some() {
                    Some(p.started_at.elapsed().as_secs())
                } else {
                    None
                },
                command: p.command.clone(),
                port: p.port,
                url: p.port.map(|port| format!("http://127.0.0.1:{}", port)),
            })
            .collect();
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        infos
    }

    fn find(&self, target: &str) -> Option<&ManagedProcess> {
        self.processes
            .get(target)
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
