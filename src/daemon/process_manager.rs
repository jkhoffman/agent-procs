use crate::daemon::log_writer::{self, OutputLine};
use crate::daemon::port_allocator::PortAllocator;
use crate::paths;
use crate::protocol::{
    ErrorCode, ProcessInfo, ProcessState, Response, RestartMode, RestartPolicy,
    Stream as ProtoStream, WatchConfig, process_url,
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
    // Supervisor fields
    pub restart_policy: Option<RestartPolicy>,
    pub watch_config: Option<WatchConfig>,
    pub restart_count: u32,
    pub manually_stopped: bool,
    pub restart_pending: bool,
    pub failed: bool,
    pub supervisor_tx: Option<tokio::sync::mpsc::Sender<String>>,
    pub capture_handles: Vec<tokio::task::JoinHandle<()>>,
    pub watch_handle: Option<crate::daemon::watcher::WatchHandle>,
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
        // Supervisor channel: stdout gets the real sender, stderr gets a dummy
        let (sup_tx, sup_rx_stdout) = tokio::sync::mpsc::channel::<String>(16);
        let (stderr_sup_sender, sup_rx_stderr) = tokio::sync::mpsc::channel::<String>(16);
        drop(stderr_sup_sender);

        let mut capture_handles = Vec::new();

        if let Some(stdout) = child.stdout.take() {
            let tx = self.output_tx.clone();
            let pname = name.clone();
            let path = log_dir.join(format!("{}.stdout", name));
            let seq = Arc::clone(&seq_counter);
            let handle = tokio::spawn(async move {
                log_writer::capture_output(
                    stdout,
                    &path,
                    &pname,
                    ProtoStream::Stdout,
                    tx,
                    DEFAULT_MAX_LOG_BYTES,
                    log_writer::DEFAULT_MAX_ROTATED_FILES,
                    seq,
                    sup_rx_stdout,
                )
                .await;
            });
            capture_handles.push(handle);
        }
        if let Some(stderr) = child.stderr.take() {
            let tx = self.output_tx.clone();
            let pname = name.clone();
            let path = log_dir.join(format!("{}.stderr", name));
            let seq = Arc::clone(&seq_counter);
            let handle = tokio::spawn(async move {
                log_writer::capture_output(
                    stderr,
                    &path,
                    &pname,
                    ProtoStream::Stderr,
                    tx,
                    DEFAULT_MAX_LOG_BYTES,
                    log_writer::DEFAULT_MAX_ROTATED_FILES,
                    seq,
                    sup_rx_stderr,
                )
                .await;
            });
            capture_handles.push(handle);
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
                restart_policy: None,
                watch_config: None,
                restart_count: 0,
                manually_stopped: false,
                restart_pending: false,
                failed: false,
                supervisor_tx: Some(sup_tx),
                capture_handles,
                watch_handle: None,
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

        proc.manually_stopped = true;

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
        let (command, name, cwd, env, port, restart_policy, watch_config) = match self.find(target)
        {
            Some(p) => (
                p.command.clone(),
                p.name.clone(),
                p.cwd.clone(),
                p.env.clone(),
                p.port,
                p.restart_policy.clone(),
                p.watch_config.clone(),
            ),
            None => {
                return Response::Error {
                    code: ErrorCode::NotFound,
                    message: format!("process not found: {}", target),
                };
            }
        };
        // Clear supervisor flags — restart is intentional
        if let Some(p) = self.find_mut(target) {
            p.manually_stopped = false;
            p.restart_count = 0;
            p.failed = false;
            p.restart_pending = false;
        }
        let _ = self.stop_process(target).await;
        self.processes.remove(&name);
        let env = if env.is_empty() { None } else { Some(env) };
        let resp = self
            .spawn_process(
                &command,
                Some(name.clone()),
                cwd.as_deref(),
                env.as_ref(),
                port,
            )
            .await;
        // Re-attach restart/watch config so manual restart preserves supervisor behavior
        if let Response::RunOk { .. } = resp
            && let Some(p) = self.find_mut(&name)
        {
            p.restart_policy = restart_policy;
            p.watch_config = watch_config;
        }
        resp
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
                } else if p.failed {
                    ProcessState::Failed
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
                restart_count: if p.restart_count > 0 {
                    Some(p.restart_count)
                } else {
                    None
                },
                max_restarts: p.restart_policy.as_ref().and_then(|rp| rp.max_restarts),
                restart_policy: p.restart_policy.as_ref().map(|rp| match rp.mode {
                    RestartMode::Always => "always".into(),
                    RestartMode::OnFailure => "on-failure".into(),
                    RestartMode::Never => "never".into(),
                }),
                watched: if p.watch_config.is_some() {
                    Some(true)
                } else {
                    None
                },
            })
            .collect();
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        infos
    }

    pub(crate) fn find(&self, target: &str) -> Option<&ManagedProcess> {
        self.processes
            .get(target)
            .or_else(|| self.processes.values().find(|p| p.id == target))
    }

    pub(crate) fn find_mut(&mut self, target: &str) -> Option<&mut ManagedProcess> {
        if self.processes.contains_key(target) {
            self.processes.get_mut(target)
        } else {
            self.processes.values_mut().find(|p| p.id == target)
        }
    }

    /// Classify exited processes that have a restart policy into "restartable" vs "exhausted".
    /// Returns `(restartable, exhausted)` name vectors in a single pass.
    pub fn classify_restart_candidates(&self) -> (Vec<String>, Vec<String>) {
        let mut restartable = Vec::new();
        let mut exhausted = Vec::new();
        for p in self.processes.values() {
            if p.child.is_some() || p.manually_stopped || p.restart_pending || p.failed {
                continue;
            }
            let Some(ref policy) = p.restart_policy else {
                continue;
            };
            if !policy.mode.should_restart(p.exit_code) {
                continue;
            }
            if policy
                .max_restarts
                .is_some_and(|max| p.restart_count >= max)
            {
                exhausted.push(p.name.clone());
            } else {
                restartable.push(p.name.clone());
            }
        }
        (restartable, exhausted)
    }

    /// Mark a process as failed (max restarts exhausted).
    pub fn mark_failed(&mut self, target: &str) {
        if let Some(p) = self.find_mut(target) {
            p.failed = true;
        }
    }

    /// Re-spawn a process in place, preserving supervisor metadata.
    /// Drains capture tasks, rotates logs, re-spawns, and carries over metadata.
    /// On spawn failure, reinserts a tombstone record with failed=true.
    pub async fn respawn_in_place(&mut self, target: &str) -> Result<(), String> {
        let proc = self
            .find(target)
            .ok_or_else(|| format!("process not found: {}", target))?;

        // 1. Save metadata and spawn args
        let command = proc.command.clone();
        let name = proc.name.clone();
        let cwd = proc.cwd.clone();
        let env = proc.env.clone();
        let port = proc.port;
        let restart_policy = proc.restart_policy.clone();
        let watch_config = proc.watch_config.clone();
        let restart_count = proc.restart_count;
        let restart_pending = proc.restart_pending;

        // 2. Drop supervisor sender (signals capture task to drain)
        if let Some(proc) = self.find_mut(target) {
            proc.supervisor_tx = None;
        }

        // 3. Await capture task JoinHandles
        if let Some(proc) = self.find_mut(target) {
            let handles = std::mem::take(&mut proc.capture_handles);
            for h in handles {
                let _ = h.await;
            }
        }

        // 4. Rotate log files
        let log_dir = crate::paths::log_dir(&self.session);
        let stdout_path = log_dir.join(format!("{}.stdout", name));
        let stderr_path = log_dir.join(format!("{}.stderr", name));
        log_writer::rotate_if_exists(&stdout_path).await;
        log_writer::rotate_if_exists(&stderr_path).await;

        // 5. Remove old record
        self.processes.remove(&name);

        // 6. Spawn fresh
        let env_opt = if env.is_empty() {
            None
        } else {
            Some(env.clone())
        };
        let resp = self
            .spawn_process(
                &command,
                Some(name.clone()),
                cwd.as_deref(),
                env_opt.as_ref(),
                port,
            )
            .await;

        match resp {
            Response::RunOk { .. } => {
                // 7. Copy metadata to new record
                if let Some(p) = self.find_mut(&name) {
                    p.restart_policy = restart_policy;
                    p.watch_config = watch_config;
                    p.restart_count = restart_count;
                    p.failed = false;
                }
                Ok(())
            }
            Response::Error { message, .. } => {
                // 8. Reinsert tombstone
                self.processes.insert(
                    name.clone(),
                    ManagedProcess {
                        name: name.clone(),
                        id: "tombstone".into(),
                        command,
                        cwd,
                        env,
                        child: None,
                        pid: 0,
                        started_at: Instant::now(),
                        exit_code: None,
                        port,
                        restart_policy,
                        watch_config,
                        restart_count,
                        manually_stopped: false,
                        restart_pending,
                        failed: true,
                        supervisor_tx: None,
                        capture_handles: Vec::new(),
                        watch_handle: None,
                    },
                );
                Err(message)
            }
            _ => Err("unexpected response from spawn".into()),
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

    #[tokio::test]
    async fn test_respawn_in_place_preserves_metadata() {
        let mut pm = ProcessManager::new("test-respawn");
        let resp = pm
            .spawn_process("echo hello", Some("worker".into()), None, None, None)
            .await;
        assert!(matches!(resp, Response::RunOk { .. }));

        // Set supervisor metadata
        if let Some(p) = pm.find_mut("worker") {
            p.restart_policy = Some(RestartPolicy {
                mode: RestartMode::OnFailure,
                max_restarts: Some(5),
                restart_delay_ms: 1000,
            });
            p.restart_count = 3;
        }

        // Wait for the process to exit
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        pm.refresh_exit_states();

        // Respawn
        let result = pm.respawn_in_place("worker").await;
        assert!(result.is_ok());

        // Verify metadata preserved
        let p = pm.find("worker").unwrap();
        assert!(p.child.is_some()); // new process running
        assert_eq!(p.restart_count, 3);
        assert!(p.restart_policy.is_some());
        assert_eq!(
            p.restart_policy.as_ref().unwrap().mode,
            RestartMode::OnFailure
        );
        assert!(!p.failed);
    }

    #[tokio::test]
    async fn test_respawn_in_place_tombstone_on_failure() {
        let mut pm = ProcessManager::new("test-tombstone");
        let resp = pm
            .spawn_process("echo hello", Some("worker".into()), None, None, None)
            .await;
        assert!(matches!(resp, Response::RunOk { .. }));

        if let Some(p) = pm.find_mut("worker") {
            p.restart_policy = Some(RestartPolicy {
                mode: RestartMode::Always,
                max_restarts: Some(3),
                restart_delay_ms: 1000,
            });
            p.restart_count = 2;
            // Corrupt name to contain path traversal — triggers spawn validation error
            p.name = "work/er".to_string();
        }

        // Re-key in the processes map under the corrupted name
        let proc = pm.processes.remove("worker").unwrap();
        pm.processes.insert("work/er".to_string(), proc);

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        pm.refresh_exit_states();

        let result = pm.respawn_in_place("work/er").await;
        assert!(result.is_err());

        // Tombstone should exist
        let p = pm.find("work/er").unwrap();
        assert!(p.child.is_none());
        assert!(p.failed);
        assert_eq!(p.restart_count, 2); // preserved
        assert!(p.restart_policy.is_some());
    }
}
