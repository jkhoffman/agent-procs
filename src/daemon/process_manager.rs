use crate::daemon::log_writer::{self, OutputLine};
use crate::daemon::port_allocator::PortAllocator;
use crate::paths;
use crate::protocol::{
    ErrorCode, ProcessInfo, ProcessState, Response, Stream as ProtoStream, process_url,
};
use crate::session::IdCounter;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use tokio::sync::broadcast;

const DEFAULT_MAX_LOG_BYTES: u64 = 50 * 1024 * 1024; // 50MB

/// Returns true if `name` is a valid DNS label: 1-63 lowercase alphanumeric/hyphen
/// chars, not starting or ending with a hyphen.
#[must_use]
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
    port_allocator: PortAllocator,
}

impl ProcessManager {
    pub fn new(session: &str) -> Self {
        let (output_tx, _) = broadcast::channel(1024);
        Self {
            processes: HashMap::new(),
            id_counter: IdCounter::new(),
            session: session.to_string(),
            output_tx,
            port_allocator: PortAllocator::new(),
        }
    }

    #[allow(unsafe_code, clippy::unused_async)]
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
                code: ErrorCode::General,
                message: format!("invalid process name: {}", name),
            };
        }

        // When proxy is active, names must be valid DNS labels for subdomain routing
        if self.port_allocator.is_proxy_enabled() && !is_valid_dns_label(&name) {
            return Response::Error {
                code: ErrorCode::General,
                message: format!(
                    "invalid process name for proxy: '{}' (must be lowercase alphanumeric/hyphens, max 63 chars)",
                    name
                ),
            };
        }

        // Resolve the port: use explicit port, auto-assign if proxy is enabled, or None
        let resolved_port = if let Some(p) = port {
            Some(p)
        } else if self.port_allocator.is_proxy_enabled() {
            let assigned: std::collections::HashSet<u16> = self
                .processes
                .values()
                .filter(|p| p.child.is_some())
                .filter_map(|p| p.port)
                .collect();
            match self.port_allocator.auto_assign_port(&assigned) {
                Ok(p) => Some(p),
                Err(e) => {
                    return Response::Error {
                        code: ErrorCode::General,
                        message: e.to_string(),
                    };
                }
            }
        } else {
            None
        };

        if self.processes.contains_key(&name) {
            return Response::Error {
                code: ErrorCode::General,
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
        // SAFETY: `setpgid(0, 0)` creates a new process group with the child as
        // leader.  This must happen before exec so that all grandchildren inherit
        // the group.  The parent uses this PGID to signal the entire tree on stop.
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
                    code: ErrorCode::General,
                    message: format!("failed to spawn: {}", e),
                };
            }
        };

        let pid = child.id().unwrap_or(0);

        // Shared sequence counter for interleaved ordering across streams
        let seq_counter = Arc::new(AtomicU64::new(0));

        // Spawn output capture tasks via log_writer
        if let Some(stdout) = child.stdout.take() {
            let tx = self.output_tx.clone();
            let pname = name.clone();
            let path = log_dir.join(format!("{}.stdout", name));
            let seq = Arc::clone(&seq_counter);
            tokio::spawn(async move {
                log_writer::capture_output(
                    stdout,
                    &path,
                    &pname,
                    ProtoStream::Stdout,
                    tx,
                    DEFAULT_MAX_LOG_BYTES,
                    log_writer::DEFAULT_MAX_ROTATED_FILES,
                    seq,
                )
                .await;
            });
        }
        if let Some(stderr) = child.stderr.take() {
            let tx = self.output_tx.clone();
            let pname = name.clone();
            let path = log_dir.join(format!("{}.stderr", name));
            let seq = Arc::clone(&seq_counter);
            tokio::spawn(async move {
                log_writer::capture_output(
                    stderr,
                    &path,
                    &pname,
                    ProtoStream::Stderr,
                    tx,
                    DEFAULT_MAX_LOG_BYTES,
                    log_writer::DEFAULT_MAX_ROTATED_FILES,
                    seq,
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
                cwd: cwd.map(std::string::ToString::to_string),
                env: env.cloned().unwrap_or_default(),
                child: Some(child),
                pid,
                started_at: Instant::now(),
                exit_code: None,
                port: resolved_port,
            },
        );

        let url = resolved_port.map(|p| process_url(&name, p, None));
        Response::RunOk {
            name,
            id,
            pid,
            port: resolved_port,
            url,
        }
    }

    pub async fn stop_process(&mut self, target: &str) -> Response {
        let proc = match self.find_mut(target) {
            Some(p) => p,
            None => {
                return Response::Error {
                    code: ErrorCode::NotFound,
                    message: format!("process not found: {}", target),
                };
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
            let _ = self.stop_process(&name).await;
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
                    code: ErrorCode::NotFound,
                    message: format!("process not found: {}", target),
                };
            }
        };
        let _ = self.stop_process(target).await;
        self.processes.remove(&name);
        let env = if env.is_empty() { None } else { Some(env) };
        self.spawn_process(&command, Some(name), cwd.as_deref(), env.as_ref(), port)
            .await
    }

    pub fn enable_proxy(&mut self) {
        self.port_allocator.enable_proxy();
    }

    pub fn status(&mut self) -> Response {
        self.refresh_exit_states();
        Response::Status {
            processes: self.build_process_infos(),
        }
    }

    /// Returns `None` if process not found or still running.
    /// Returns `Some(exit_code)` if process has exited (`exit_code` is None for signal kills).
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

    pub(crate) fn refresh_exit_states(&mut self) -> bool {
        let mut changed = false;
        for proc in self.processes.values_mut() {
            if proc.child.is_some()
                && proc.exit_code.is_none()
                && let Some(ref mut child) = proc.child
                && let Ok(Some(status)) = child.try_wait()
            {
                proc.exit_code = status.code();
                proc.child = None;
                changed = true;
            }
        }
        changed
    }

    pub fn session_name(&self) -> &str {
        &self.session
    }

    pub fn has_process(&self, target: &str) -> bool {
        self.find(target).is_some()
    }

    /// Returns a map of running process names to their assigned ports.
    pub fn running_ports(&self) -> HashMap<String, u16> {
        self.processes
            .iter()
            .filter_map(|(name, p)| {
                if p.child.is_some() {
                    p.port.map(|port| (name.clone(), port))
                } else {
                    None
                }
            })
            .collect()
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
                url: p.port.map(|port| process_url(&p.name, port, None)),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_dns_labels() {
        assert!(is_valid_dns_label("api"));
        assert!(is_valid_dns_label("my-app"));
        assert!(is_valid_dns_label("a"));
        assert!(is_valid_dns_label("a1"));
        assert!(is_valid_dns_label("123"));
    }

    #[test]
    fn test_invalid_dns_labels() {
        assert!(!is_valid_dns_label(""));
        assert!(!is_valid_dns_label("-start"));
        assert!(!is_valid_dns_label("end-"));
        assert!(!is_valid_dns_label("UPPER"));
        assert!(!is_valid_dns_label("has.dot"));
        assert!(!is_valid_dns_label("has space"));
        assert!(!is_valid_dns_label(&"a".repeat(64))); // > 63 chars
        assert!(!is_valid_dns_label("has_underscore"));
    }
}
