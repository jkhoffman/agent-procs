use crate::daemon::log_writer::{self, LogSink, OutputLine};
use crate::daemon::port_allocator::PortAllocator;
use crate::paths;
use crate::protocol::{
    ErrorCode, ProcessInfo, ProcessState, Response, RestartMode, RestartPolicy,
    Stream as ProtoStream, WatchConfig, process_url,
};
use crate::session::IdCounter;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use std::collections::HashMap;
use std::num::NonZeroI32;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

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

/// How long a stopping group gets between SIGTERM and SIGKILL.
pub const STOP_GRACE: Duration = Duration::from_secs(2);
/// Bounds the final reap so a child stuck in uninterruptible sleep cannot
/// hold a stop open forever. A child still unreaped then is left to tokio's
/// orphan reaper.
const REAP_CAP: Duration = Duration::from_secs(10);
/// How long capture tasks get to drain buffered output after the group dies.
const CAPTURE_DRAIN: Duration = Duration::from_millis(500);
/// How long cancelled capture tasks get to flush before being aborted.
const CAPTURE_CANCEL: Duration = Duration::from_millis(100);
const EXIT_POLL: Duration = Duration::from_millis(50);

/// The id of a process group this daemon created. Zero is unrepresentable
/// because `killpg(0, ..)` would signal the daemon's own group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pgid(NonZeroI32);

impl Pgid {
    fn from_child(child: &Child) -> Option<Self> {
        child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .and_then(NonZeroI32::new)
            .map(Self)
    }

    fn signal(self, sig: Signal) {
        let _ = killpg(Pid::from_raw(self.0.get()), sig);
    }

    /// Reports the leader's exit without reaping it. While the zombie is
    /// unreaped the kernel cannot reuse its pid, so signalling this group
    /// later cannot hit an unrelated one.
    #[allow(unsafe_code, clippy::option_option)]
    fn peek_exit(self) -> Option<Option<i32>> {
        let Ok(id) = libc::id_t::try_from(self.0.get()) else {
            return Some(None);
        };
        // SAFETY: siginfo_t is plain data; waitid only writes into it.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: valid pointer to an owned siginfo_t. WNOWAIT leaves the
        // child waitable, so tokio's later wait() still reaps it.
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                id,
                &raw mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ECHILD) {
                // Reaped behind our back (e.g. an inherited SIGCHLD=SIG_IGN),
                // so the leader is gone either way.
                return Some(None);
            }
            tracing::warn!(pid = self.0.get(), error = %err, "waitid failed");
            return None;
        }
        // SAFETY: waitid filled in the SIGCHLD fields.
        let (pid, status) = unsafe { (info.si_pid(), info.si_status()) };
        if pid == 0 {
            return None;
        }
        Some((info.si_code == libc::CLD_EXITED).then_some(status))
    }
}

/// One spawn of a managed command. The leader is never reaped outside
/// [`teardown`], so a record's pgid stays safe to signal while it exists.
pub struct Incarnation {
    pgid: Pgid,
    child: Child,
    capture: Vec<tokio::task::JoinHandle<()>>,
    cancel_capture: watch::Sender<bool>,
    supervisor_tx: Option<mpsc::Sender<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitWhy {
    Natural,
    Manual,
    Exhausted,
    SpawnFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RespawnReason {
    Manual,
    Auto,
    Watch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterStop {
    Park,
    Respawn(RespawnReason),
}

/// A client waiting on an in-flight stop.
pub enum Waiter {
    Stop(oneshot::Sender<Response>),
    Restart(oneshot::Sender<Response>),
}

pub enum Life {
    Running(Incarnation),
    /// `inc` is `Some` while the leader is an unreaped zombie whose group or
    /// capture tasks may still be alive.
    Exited {
        code: Option<i32>,
        why: ExitWhy,
        inc: Option<Incarnation>,
        restart_timer: bool,
    },
    /// A teardown task owns the incarnation. `leader_exit` is `Some` when the
    /// leader had already exited before the stop began.
    Stopping {
        leader_exit: Option<Option<i32>>,
        then: AfterStop,
        waiters: Vec<Waiter>,
    },
}

impl Life {
    pub(crate) fn vacant() -> Self {
        Self::Exited {
            code: None,
            why: ExitWhy::SpawnFailed,
            inc: None,
            restart_timer: false,
        }
    }

    fn holds_port(&self) -> bool {
        matches!(self, Self::Running(_) | Self::Stopping { .. })
    }

    fn leader_alive(&self) -> bool {
        matches!(
            self,
            Self::Running(_)
                | Self::Stopping {
                    leader_exit: None,
                    ..
                }
        )
    }
}

pub struct ManagedProcess {
    pub name: String,
    pub id: String,
    pub command: String,
    pub cwd: Option<String>,
    pub env: HashMap<String, String>,
    pub pid: u32,
    pub started_at: Instant,
    pub port: Option<u16>,
    pub restart_policy: Option<RestartPolicy>,
    pub watch_config: Option<WatchConfig>,
    pub restart_count: u32,
    /// Daemon-wide spawn generation; stale timers, watch events, and teardown
    /// results carry the generation they were issued for.
    pub generation: u64,
    pub life: Life,
    pub watch_handle: Option<crate::daemon::watcher::WatchHandle>,
}

impl ManagedProcess {
    pub fn supervisor_tx(&self) -> Option<&mpsc::Sender<String>> {
        match &self.life {
            Life::Running(inc) | Life::Exited { inc: Some(inc), .. } => inc.supervisor_tx.as_ref(),
            _ => None,
        }
    }
}

/// SIGTERM the group, give it [`STOP_GRACE`], SIGKILL the group, then reap the
/// leader and drain capture. Returns the leader's exit code, or `-9` when it
/// had to be killed.
pub async fn teardown(mut inc: Incarnation, mut leader_exit: Option<Option<i32>>) -> Option<i32> {
    inc.supervisor_tx = None;
    inc.pgid.signal(Signal::SIGTERM);
    let deadline = Instant::now() + STOP_GRACE;
    while Instant::now() < deadline {
        if leader_exit.is_none() {
            leader_exit = inc.pgid.peek_exit();
        }
        if leader_exit.is_some() && inc.capture.iter().all(tokio::task::JoinHandle::is_finished) {
            break;
        }
        tokio::time::sleep(EXIT_POLL).await;
    }
    // Unconditional: members can outlive a cooperative leader, and the
    // unreaped leader still pins the pgid.
    inc.pgid.signal(Signal::SIGKILL);
    let reaped = tokio::time::timeout(REAP_CAP, inc.child.wait()).await;
    let code = leader_exit.unwrap_or_else(|| match reaped {
        Ok(Ok(status)) => status.code().or(Some(-9)),
        _ => Some(-9),
    });
    join_capture(std::mem::take(&mut inc.capture), &inc.cancel_capture).await;
    code
}

async fn join_capture(handles: Vec<tokio::task::JoinHandle<()>>, cancel: &watch::Sender<bool>) {
    let mut pending = Vec::new();
    let deadline = tokio::time::Instant::now() + CAPTURE_DRAIN;
    for mut h in handles {
        if tokio::time::timeout_at(deadline, &mut h).await.is_err() {
            pending.push(h);
        }
    }
    if pending.is_empty() {
        return;
    }
    let _ = cancel.send(true);
    let deadline = tokio::time::Instant::now() + CAPTURE_CANCEL;
    for mut h in pending {
        if tokio::time::timeout_at(deadline, &mut h).await.is_err() {
            h.abort();
            let _ = h.await;
        }
    }
}

pub struct ProcessManager {
    processes: HashMap<String, ManagedProcess>,
    id_counter: IdCounter,
    next_generation: u64,
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
            next_generation: 0,
            session: session.to_string(),
            output_tx,
            port_allocator: PortAllocator::new(),
        }
    }

    fn bump_generation(&mut self) -> u64 {
        self.next_generation += 1;
        self.next_generation
    }

    #[allow(unsafe_code)]
    fn spawn_incarnation(
        &self,
        name: &str,
        command: &str,
        cwd: Option<&str>,
        env: Option<&HashMap<String, String>>,
        port: Option<u16>,
    ) -> Result<(Incarnation, u32), String> {
        let log_dir = paths::log_dir(&self.session);
        let _ = std::fs::create_dir_all(&log_dir);
        let stdout_path = log_dir.join(format!("{}.stdout", name));
        let stderr_path = log_dir.join(format!("{}.stderr", name));
        let stdout_sink = LogSink::open(&stdout_path, 0)?;
        let stderr_sink = LogSink::open(&stderr_path, 0)?;

        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        if let Some(p) = port {
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
                nix::unistd::setpgid(Pid::from_raw(0), Pid::from_raw(0))
                    .map_err(std::io::Error::other)?;
                Ok(())
            });
        }

        let mut child = cmd.spawn().map_err(|e| format!("failed to spawn: {}", e))?;
        let group = Pgid::from_child(&child).ok_or("spawned process has no pid")?;
        let pid = child.id().unwrap_or(0);

        // Shared sequence counter for interleaved ordering across streams
        let seq_counter = Arc::new(AtomicU64::new(0));
        // Supervisor channel: stdout gets the real sender, stderr gets a dummy
        let (sup_tx, sup_rx_stdout) = mpsc::channel::<String>(16);
        let (_, sup_rx_stderr) = mpsc::channel::<String>(16);
        let (cancel_capture, cancel_rx) = watch::channel(false);

        let mut capture = Vec::new();
        let streams = [
            (
                child
                    .stdout
                    .take()
                    .map(|r| Box::new(r) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
                stdout_sink,
                stdout_path,
                ProtoStream::Stdout,
                sup_rx_stdout,
            ),
            (
                child
                    .stderr
                    .take()
                    .map(|r| Box::new(r) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
                stderr_sink,
                stderr_path,
                ProtoStream::Stderr,
                sup_rx_stderr,
            ),
        ];
        for (reader, sink, path, stream, sup_rx) in streams {
            let Some(reader) = reader else { continue };
            let tx = self.output_tx.clone();
            let pname = name.to_string();
            let seq = Arc::clone(&seq_counter);
            let cancel = cancel_rx.clone();
            capture.push(tokio::spawn(async move {
                log_writer::capture_output(
                    reader,
                    sink,
                    &path,
                    &pname,
                    stream,
                    tx,
                    DEFAULT_MAX_LOG_BYTES,
                    log_writer::DEFAULT_MAX_ROTATED_FILES,
                    seq,
                    sup_rx,
                    cancel,
                )
                .await;
            }));
        }

        Ok((
            Incarnation {
                pgid: group,
                child,
                capture,
                cancel_capture,
                supervisor_tx: Some(sup_tx),
            },
            pid,
        ))
    }

    pub fn spawn_process(
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
                .filter(|p| p.life.holds_port())
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

        let (inc, pid) = match self.spawn_incarnation(&name, command, cwd, env, resolved_port) {
            Ok(spawned) => spawned,
            Err(message) => {
                return Response::Error {
                    code: ErrorCode::General,
                    message,
                };
            }
        };
        let generation = self.bump_generation();

        self.processes.insert(
            name.clone(),
            ManagedProcess {
                name: name.clone(),
                id: id.clone(),
                command: command.to_string(),
                cwd: cwd.map(std::string::ToString::to_string),
                env: env.cloned().unwrap_or_default(),
                pid,
                started_at: Instant::now(),
                port: resolved_port,
                restart_policy: None,
                watch_config: None,
                restart_count: 0,
                generation,
                life: Life::Running(inc),
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

    /// Spawn a fresh incarnation into an existing record whose previous one
    /// has been torn down. `rotate` keeps the previous incarnation's logs.
    pub async fn respawn(&mut self, name: &str, rotate: bool) -> Response {
        let Some(p) = self.processes.get(name) else {
            return Response::Error {
                code: ErrorCode::NotFound,
                message: format!("process not found: {}", name),
            };
        };
        let (command, cwd, env, port) = (p.command.clone(), p.cwd.clone(), p.env.clone(), p.port);
        if rotate {
            let log_dir = paths::log_dir(&self.session);
            log_writer::rotate_if_exists(&log_dir.join(format!("{}.stdout", name))).await;
            log_writer::rotate_if_exists(&log_dir.join(format!("{}.stderr", name))).await;
        }
        let env_opt = (!env.is_empty()).then_some(&env);
        let spawned = self.spawn_incarnation(name, &command, cwd.as_deref(), env_opt, port);
        let generation = self.bump_generation();
        let id = self.id_counter.next_id();
        let Some(p) = self.processes.get_mut(name) else {
            unreachable!("record checked above");
        };
        match spawned {
            Ok((inc, pid)) => {
                p.id.clone_from(&id);
                p.pid = pid;
                p.started_at = Instant::now();
                p.generation = generation;
                p.life = Life::Running(inc);
                Response::RunOk {
                    name: name.to_string(),
                    id,
                    pid,
                    port,
                    url: port.map(|port| process_url(name, port, None)),
                }
            }
            Err(message) => {
                p.pid = 0;
                let code = match p.life {
                    Life::Exited { code, .. } => code,
                    _ => None,
                };
                p.life = Life::Exited {
                    code,
                    why: ExitWhy::SpawnFailed,
                    inc: None,
                    restart_timer: false,
                };
                Response::Error {
                    code: ErrorCode::General,
                    message,
                }
            }
        }
    }

    pub fn enable_proxy(&mut self) {
        self.port_allocator.enable_proxy();
    }

    /// Returns `None` if process not found or still running.
    /// Returns `Some(exit_code)` if process has exited (`exit_code` is None for signal kills).
    pub fn is_process_exited(&mut self, target: &str) -> Option<Option<i32>> {
        self.refresh_exit_states();
        self.find(target).and_then(|p| match &p.life {
            Life::Running(_)
            | Life::Stopping {
                leader_exit: None, ..
            } => None,
            Life::Exited { code, .. }
            | Life::Stopping {
                leader_exit: Some(code),
                ..
            } => Some(*code),
        })
    }

    pub(crate) fn refresh_exit_states(&mut self) -> bool {
        let mut changed = false;
        for proc in self.processes.values_mut() {
            let Life::Running(inc) = &proc.life else {
                continue;
            };
            let Some(code) = inc.pgid.peek_exit() else {
                continue;
            };
            let Life::Running(inc) = std::mem::replace(&mut proc.life, Life::vacant()) else {
                unreachable!("matched Running above");
            };
            proc.life = Life::Exited {
                code,
                why: ExitWhy::Natural,
                inc: Some(inc),
                restart_timer: false,
            };
            changed = true;
        }
        changed
    }

    pub fn session_name(&self) -> &str {
        &self.session
    }

    pub fn has_process(&self, target: &str) -> bool {
        self.find(target).is_some()
    }

    pub fn names(&self) -> Vec<String> {
        self.processes.keys().cloned().collect()
    }

    pub fn clear(&mut self) {
        self.processes.clear();
    }

    /// Returns a map of running process names to their assigned ports.
    pub fn running_ports(&self) -> HashMap<String, u16> {
        self.processes
            .iter()
            .filter(|(_, p)| p.life.leader_alive())
            .filter_map(|(name, p)| p.port.map(|port| (name.clone(), port)))
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
            .map(|p| {
                let running = p.life.leader_alive();
                let (state, exit_code) = match &p.life {
                    _ if running => (ProcessState::Running, None),
                    Life::Exited {
                        code,
                        why: ExitWhy::Exhausted | ExitWhy::SpawnFailed,
                        ..
                    } => (ProcessState::Failed, *code),
                    Life::Exited { code, .. }
                    | Life::Stopping {
                        leader_exit: Some(code),
                        ..
                    } => (ProcessState::Exited, *code),
                    Life::Running(_) | Life::Stopping { .. } => (ProcessState::Running, None),
                };
                ProcessInfo {
                    name: p.name.clone(),
                    id: p.id.clone(),
                    pid: p.pid,
                    state,
                    exit_code,
                    uptime_secs: running.then(|| p.started_at.elapsed().as_secs()),
                    command: p.command.clone(),
                    port: p.port,
                    url: p.port.map(|port| process_url(&p.name, port, None)),
                    restart_count: (p.restart_count > 0).then_some(p.restart_count),
                    max_restarts: p.restart_policy.as_ref().and_then(|rp| rp.max_restarts),
                    restart_policy: p.restart_policy.as_ref().map(|rp| match rp.mode {
                        RestartMode::Always => "always".into(),
                        RestartMode::OnFailure => "on-failure".into(),
                        RestartMode::Never => "never".into(),
                    }),
                    watched: p.watch_config.is_some().then_some(true),
                }
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
            let Life::Exited {
                code,
                why: ExitWhy::Natural,
                restart_timer: false,
                ..
            } = p.life
            else {
                continue;
            };
            let Some(ref policy) = p.restart_policy else {
                continue;
            };
            if !policy.mode.should_restart(code) {
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
        if let Some(p) = self.find_mut(target)
            && let Life::Exited { why, .. } = &mut p.life
        {
            *why = ExitWhy::Exhausted;
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

    async fn wait_exited(pm: &mut ProcessManager, name: &str) {
        for _ in 0..100 {
            pm.refresh_exit_states();
            if matches!(pm.find(name).unwrap().life, Life::Exited { .. }) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("{name} never exited");
    }

    fn take_exited_incarnation(pm: &mut ProcessManager, name: &str) -> (Incarnation, Option<i32>) {
        let p = pm.find_mut(name).unwrap();
        match std::mem::replace(&mut p.life, Life::vacant()) {
            Life::Exited {
                code,
                inc: Some(inc),
                ..
            } => (inc, code),
            _ => panic!("{name} is not an exited incarnation"),
        }
    }

    #[tokio::test]
    async fn test_exit_is_observed_without_reaping() {
        let mut pm = ProcessManager::new("test-peek");
        let resp = pm.spawn_process("exit 3", Some("worker".into()), None, None, None);
        assert!(matches!(resp, Response::RunOk { .. }));
        wait_exited(&mut pm, "worker").await;
        assert_eq!(pm.is_process_exited("worker"), Some(Some(3)));

        let (inc, code) = take_exited_incarnation(&mut pm, "worker");
        let pgid = inc.pgid;
        assert_eq!(
            pgid.peek_exit(),
            Some(Some(3)),
            "leader stays waitable after a peek"
        );
        assert_eq!(teardown(inc, Some(code)).await, Some(3));
        assert!(
            killpg(Pid::from_raw(pgid.0.get()), None).is_err(),
            "teardown reaps the leader"
        );
    }

    #[tokio::test]
    async fn test_respawn_preserves_metadata() {
        let mut pm = ProcessManager::new("test-respawn");
        let resp = pm.spawn_process("echo hello", Some("worker".into()), None, None, None);
        assert!(matches!(resp, Response::RunOk { .. }));

        if let Some(p) = pm.find_mut("worker") {
            p.restart_policy = Some(RestartPolicy {
                mode: RestartMode::OnFailure,
                max_restarts: Some(5),
                restart_delay_ms: 1000,
            });
            p.restart_count = 3;
        }
        wait_exited(&mut pm, "worker").await;
        let old_generation = pm.find("worker").unwrap().generation;
        let (inc, code) = take_exited_incarnation(&mut pm, "worker");
        teardown(inc, Some(code)).await;

        let resp = pm.respawn("worker", true).await;
        assert!(matches!(resp, Response::RunOk { .. }), "{resp:?}");
        let p = pm.find("worker").unwrap();
        assert!(matches!(p.life, Life::Running(_)));
        assert!(p.generation > old_generation);
        assert_eq!(p.restart_count, 3);
        assert_eq!(
            p.restart_policy.as_ref().unwrap().mode,
            RestartMode::OnFailure
        );
    }

    #[tokio::test]
    async fn test_respawn_failure_keeps_failed_record() {
        let mut pm = ProcessManager::new("test-respawn-fail");
        let log = paths::log_dir("test-respawn-fail").join("worker.stdout");
        let _ = std::fs::remove_dir(&log);
        let resp = pm.spawn_process("true", Some("worker".into()), None, None, None);
        assert!(matches!(resp, Response::RunOk { .. }));
        if let Some(p) = pm.find_mut("worker") {
            p.restart_count = 2;
        }
        wait_exited(&mut pm, "worker").await;
        let (inc, code) = take_exited_incarnation(&mut pm, "worker");
        teardown(inc, Some(code)).await;

        std::fs::remove_file(&log).unwrap();
        std::fs::create_dir(&log).unwrap();
        let resp = pm.respawn("worker", false).await;
        std::fs::remove_dir(&log).unwrap();
        assert!(matches!(resp, Response::Error { .. }), "{resp:?}");

        let p = pm.find("worker").unwrap();
        assert!(matches!(
            p.life,
            Life::Exited {
                why: ExitWhy::SpawnFailed,
                inc: None,
                ..
            }
        ));
        assert_eq!(p.restart_count, 2);
        let Response::Status { processes } = pm.status_snapshot() else {
            panic!("status");
        };
        assert_eq!(processes[0].state, ProcessState::Failed);
    }

    /// Helper: build an exited `ManagedProcess` with sensible defaults.
    fn make_exited_process(name: &str, exit_code: Option<i32>) -> ManagedProcess {
        ManagedProcess {
            name: name.to_string(),
            id: format!("id-{}", name),
            command: "true".to_string(),
            cwd: None,
            env: HashMap::new(),
            pid: 0,
            started_at: Instant::now(),
            port: None,
            restart_policy: None,
            watch_config: None,
            restart_count: 0,
            generation: 0,
            life: Life::Exited {
                code: exit_code,
                why: ExitWhy::Natural,
                inc: None,
                restart_timer: false,
            },
            watch_handle: None,
        }
    }

    #[tokio::test]
    async fn test_classify_skips_running_process() {
        let mut pm = ProcessManager::new("test-classify-running");
        let resp = pm.spawn_process("sleep 60", Some("runner".into()), None, None, None);
        assert!(matches!(resp, Response::RunOk { .. }));
        pm.find_mut("runner").unwrap().restart_policy = Some(RestartPolicy {
            mode: RestartMode::Always,
            max_restarts: None,
            restart_delay_ms: 0,
        });

        let (restartable, exhausted) = pm.classify_restart_candidates();
        assert!(restartable.is_empty(), "{restartable:?}");
        assert!(exhausted.is_empty(), "{exhausted:?}");

        let p = pm.find_mut("runner").unwrap();
        let Life::Running(inc) = std::mem::replace(&mut p.life, Life::vacant()) else {
            panic!("runner should be running");
        };
        teardown(inc, None).await;
    }

    #[test]
    fn test_classify_skips_manually_stopped() {
        let mut pm = ProcessManager::new("test-classify-stopped");
        let mut proc = make_exited_process("stopped", Some(1));
        proc.restart_policy = Some(RestartPolicy {
            mode: RestartMode::Always,
            max_restarts: None,
            restart_delay_ms: 0,
        });
        proc.life = Life::Exited {
            code: Some(1),
            why: ExitWhy::Manual,
            inc: None,
            restart_timer: false,
        };
        pm.processes.insert("stopped".into(), proc);

        let (restartable, exhausted) = pm.classify_restart_candidates();
        assert!(restartable.is_empty(), "{restartable:?}");
        assert!(exhausted.is_empty(), "{exhausted:?}");
    }

    #[test]
    fn test_classify_skips_clean_exit_on_failure_mode() {
        let mut pm = ProcessManager::new("test-classify-clean");
        let mut proc = make_exited_process("clean", Some(0));
        proc.restart_policy = Some(RestartPolicy {
            mode: RestartMode::OnFailure,
            max_restarts: None,
            restart_delay_ms: 0,
        });
        pm.processes.insert("clean".into(), proc);

        let (restartable, exhausted) = pm.classify_restart_candidates();
        assert!(restartable.is_empty(), "{restartable:?}");
        assert!(exhausted.is_empty(), "{exhausted:?}");
    }

    #[test]
    fn test_classify_skips_restart_pending() {
        let mut pm = ProcessManager::new("test-classify-pending");
        let mut proc = make_exited_process("pending", Some(1));
        proc.restart_policy = Some(RestartPolicy {
            mode: RestartMode::Always,
            max_restarts: None,
            restart_delay_ms: 0,
        });
        proc.life = Life::Exited {
            code: Some(1),
            why: ExitWhy::Natural,
            inc: None,
            restart_timer: true,
        };
        pm.processes.insert("pending".into(), proc);

        let (restartable, exhausted) = pm.classify_restart_candidates();
        assert!(restartable.is_empty(), "{restartable:?}");
        assert!(exhausted.is_empty(), "{exhausted:?}");
    }

    #[test]
    fn test_classify_restartable_on_failure() {
        let mut pm = ProcessManager::new("test-classify-on-failure");
        let mut proc = make_exited_process("crasher", Some(1));
        proc.restart_policy = Some(RestartPolicy {
            mode: RestartMode::OnFailure,
            max_restarts: None,
            restart_delay_ms: 0,
        });
        pm.processes.insert("crasher".into(), proc);

        let (restartable, exhausted) = pm.classify_restart_candidates();
        assert_eq!(restartable, vec!["crasher"]);
        assert!(exhausted.is_empty(), "{exhausted:?}");
    }

    #[test]
    fn test_classify_exhausted() {
        let mut pm = ProcessManager::new("test-classify-exhausted");
        let mut proc = make_exited_process("tired", Some(1));
        proc.restart_policy = Some(RestartPolicy {
            mode: RestartMode::Always,
            max_restarts: Some(3),
            restart_delay_ms: 0,
        });
        proc.restart_count = 3;
        pm.processes.insert("tired".into(), proc);

        let (restartable, exhausted) = pm.classify_restart_candidates();
        assert!(restartable.is_empty(), "{restartable:?}");
        assert_eq!(exhausted, vec!["tired"]);
    }

    #[test]
    fn test_classify_always_mode_clean_exit() {
        let mut pm = ProcessManager::new("test-classify-always-clean");
        let mut proc = make_exited_process("always-on", Some(0));
        proc.restart_policy = Some(RestartPolicy {
            mode: RestartMode::Always,
            max_restarts: None,
            restart_delay_ms: 0,
        });
        pm.processes.insert("always-on".into(), proc);

        let (restartable, exhausted) = pm.classify_restart_candidates();
        assert_eq!(restartable, vec!["always-on"]);
        assert!(exhausted.is_empty(), "{exhausted:?}");
    }
}
