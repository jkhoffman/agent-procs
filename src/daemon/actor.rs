use crate::daemon::log_writer::OutputLine;
use crate::daemon::process_manager::{
    self, AfterStop, ExitWhy, Life, ProcessManager, RespawnReason, Waiter,
};
use crate::protocol::{self, ErrorCode, Response, Stream as ProtoStream};
use std::collections::{HashMap, HashSet};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::{self, Duration, MissedTickBehavior};

const EXIT_REFRESH_INTERVAL: Duration = Duration::from_millis(200);

/// State published via the watch channel for lock-free proxy reads.
#[derive(Debug, Clone, PartialEq)]
pub struct ProxyState {
    /// Running process name → backend port.
    pub port_map: HashMap<String, u16>,
}

/// Commands sent to the actor via [`PmHandle`].
enum PmCommand {
    Spawn {
        command: String,
        name: Option<String>,
        cwd: Option<String>,
        env: Option<HashMap<String, String>>,
        port: Option<u16>,
        restart: Option<crate::protocol::RestartPolicy>,
        watch: Option<crate::protocol::WatchConfig>,
        reply: oneshot::Sender<Response>,
    },
    Stop {
        target: String,
        reply: oneshot::Sender<Response>,
    },
    StopAll {
        reply: oneshot::Sender<Response>,
    },
    Restart {
        target: String,
        reply: oneshot::Sender<Response>,
    },
    Status {
        reply: oneshot::Sender<Response>,
    },
    StatusSnapshot {
        reply: oneshot::Sender<Response>,
    },
    HasProcess {
        target: String,
        reply: oneshot::Sender<bool>,
    },
    SessionName {
        reply: oneshot::Sender<String>,
    },
    #[allow(clippy::option_option)]
    IsProcessExited {
        target: String,
        reply: oneshot::Sender<Option<Option<i32>>>,
    },
    /// Returns `Some(existing_port)` if proxy is already enabled, `None` if newly enabled.
    EnableProxy {
        proxy_port: u16,
        reply: oneshot::Sender<Option<u16>>,
    },
    Subscribe {
        reply: oneshot::Sender<broadcast::Receiver<OutputLine>>,
    },
    /// Internal: delayed auto-restart for a crashed process.
    AutoRestart {
        name: String,
        generation: u64,
    },
    /// Internal: file watcher triggered restart.
    WatchRestart {
        name: String,
        generation: u64,
    },
    /// Internal: a teardown task finished with the leader's exit code.
    StopFinished {
        name: String,
        generation: u64,
        code: Option<i32>,
    },
}

/// A `stop-all` waiting for the teardowns it started.
struct StopAllBatch {
    remaining: HashSet<String>,
    replies: Vec<oneshot::Sender<Response>>,
}

fn stop_all_in_progress() -> Response {
    actor_error("stop-all in progress")
}

fn not_found(target: &str) -> Response {
    Response::Error {
        code: ErrorCode::NotFound,
        message: format!("process not found: {}", target),
    }
}

fn stopped(name: &str) -> Response {
    Response::Ok {
        message: format!("stopped {}", name),
    }
}

fn actor_error(msg: &str) -> Response {
    Response::Error {
        code: ErrorCode::General,
        message: msg.into(),
    }
}

/// Cloneable handle for sending commands to the [`ProcessManagerActor`].
#[derive(Clone)]
pub struct PmHandle {
    tx: mpsc::Sender<PmCommand>,
}

impl PmHandle {
    pub async fn spawn_process(
        &self,
        command: String,
        name: Option<String>,
        cwd: Option<String>,
        env: Option<HashMap<String, String>>,
        port: Option<u16>,
    ) -> Response {
        self.spawn_process_supervised(command, name, cwd, env, port, None, None)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_process_supervised(
        &self,
        command: String,
        name: Option<String>,
        cwd: Option<String>,
        env: Option<HashMap<String, String>>,
        port: Option<u16>,
        restart: Option<crate::protocol::RestartPolicy>,
        watch: Option<crate::protocol::WatchConfig>,
    ) -> Response {
        let (reply, rx) = oneshot::channel();
        let _ = self
            .tx
            .send(PmCommand::Spawn {
                command,
                name,
                cwd,
                env,
                port,
                restart,
                watch,
                reply,
            })
            .await;
        rx.await.unwrap_or_else(|_| actor_error("actor stopped"))
    }

    pub async fn stop_process(&self, target: &str) -> Response {
        let (reply, rx) = oneshot::channel();
        let _ = self
            .tx
            .send(PmCommand::Stop {
                target: target.to_string(),
                reply,
            })
            .await;
        rx.await.unwrap_or_else(|_| actor_error("actor stopped"))
    }

    pub async fn stop_all(&self) -> Response {
        let (reply, rx) = oneshot::channel();
        let _ = self.tx.send(PmCommand::StopAll { reply }).await;
        rx.await.unwrap_or_else(|_| actor_error("actor stopped"))
    }

    pub async fn restart_process(&self, target: &str) -> Response {
        let (reply, rx) = oneshot::channel();
        let _ = self
            .tx
            .send(PmCommand::Restart {
                target: target.to_string(),
                reply,
            })
            .await;
        rx.await.unwrap_or_else(|_| actor_error("actor stopped"))
    }

    pub async fn status(&self) -> Response {
        let (reply, rx) = oneshot::channel();
        let _ = self.tx.send(PmCommand::Status { reply }).await;
        rx.await.unwrap_or_else(|_| actor_error("actor stopped"))
    }

    pub async fn status_snapshot(&self) -> Response {
        let (reply, rx) = oneshot::channel();
        let _ = self.tx.send(PmCommand::StatusSnapshot { reply }).await;
        rx.await.unwrap_or_else(|_| actor_error("actor stopped"))
    }

    pub async fn has_process(&self, target: &str) -> bool {
        let (reply, rx) = oneshot::channel();
        let _ = self
            .tx
            .send(PmCommand::HasProcess {
                target: target.to_string(),
                reply,
            })
            .await;
        rx.await.unwrap_or(false)
    }

    pub async fn session_name(&self) -> String {
        let (reply, rx) = oneshot::channel();
        let _ = self.tx.send(PmCommand::SessionName { reply }).await;
        rx.await.unwrap_or_default()
    }

    pub async fn is_process_exited(&self, target: &str) -> Option<Option<i32>> {
        let (reply, rx) = oneshot::channel();
        let _ = self
            .tx
            .send(PmCommand::IsProcessExited {
                target: target.to_string(),
                reply,
            })
            .await;
        rx.await.unwrap_or(None)
    }

    /// Enable proxy with the given port. Returns `Some(existing_port)` if already enabled.
    pub async fn enable_proxy(&self, proxy_port: u16) -> Option<u16> {
        let (reply, rx) = oneshot::channel();
        let _ = self
            .tx
            .send(PmCommand::EnableProxy { proxy_port, reply })
            .await;
        rx.await.unwrap_or(None)
    }

    pub async fn subscribe(&self) -> broadcast::Receiver<OutputLine> {
        let (reply, rx) = oneshot::channel();
        let _ = self.tx.send(PmCommand::Subscribe { reply }).await;
        rx.await.expect("actor should be alive for subscribe")
    }
}

/// Actor that owns the [`ProcessManager`] and processes commands sequentially.
pub struct ProcessManagerActor {
    pm: ProcessManager,
    rx: mpsc::Receiver<PmCommand>,
    /// Clone of the sender for scheduling internal commands (e.g. delayed restart).
    self_tx: mpsc::Sender<PmCommand>,
    proxy_state_tx: watch::Sender<ProxyState>,
    proxy_port: Option<u16>,
    stop_all: Option<StopAllBatch>,
}

impl ProcessManagerActor {
    /// Create the actor, its handle, and the proxy state watch channel.
    pub fn new(session: &str) -> (PmHandle, watch::Receiver<ProxyState>, Self) {
        let (tx, rx) = mpsc::channel(256);
        let pm = ProcessManager::new(session);

        let initial_state = ProxyState {
            port_map: HashMap::new(),
        };
        let (proxy_state_tx, proxy_state_rx) = watch::channel(initial_state);

        let handle = PmHandle { tx: tx.clone() };
        let actor = Self {
            pm,
            rx,
            self_tx: tx,
            proxy_state_tx,
            proxy_port: None,
            stop_all: None,
        };

        (handle, proxy_state_rx, actor)
    }

    /// Run the actor loop until all senders are dropped.
    pub async fn run(mut self) {
        let mut exit_refresh = time::interval(EXIT_REFRESH_INTERVAL);
        exit_refresh.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                cmd = self.rx.recv() => match cmd {
                    Some(cmd) => self.handle_command(cmd).await,
                    None => break,
                },
                _ = exit_refresh.tick() => {
                    if self.pm.refresh_exit_states() {
                        self.publish_proxy_state();
                    }
                    // Check for processes needing restart
                    self.schedule_restarts();
                }
            }
        }
    }

    /// Check for exited processes eligible for auto-restart, schedule delayed restarts.
    /// Also detects processes that have exhausted their `max_restarts` and marks them failed.
    fn schedule_restarts(&mut self) {
        if self.stop_all.is_some() {
            return;
        }
        let (restartable, exhausted) = self.pm.classify_restart_candidates();

        for name in &exhausted {
            if let Some(p) = self.pm.find(name)
                && let Some(ref policy) = p.restart_policy
                && let Some(max) = policy.max_restarts
                && let Some(tx) = p.supervisor_tx()
            {
                let msg = format!("[agent-procs] Max restarts ({}) exhausted", max);
                let _ = tx.try_send(msg);
            }
            self.pm.mark_failed(name);
        }
        if !exhausted.is_empty() {
            self.publish_proxy_state();
        }

        for name in restartable {
            if let Some(p) = self.pm.find_mut(&name)
                && let Life::Exited { restart_timer, .. } = &mut p.life
            {
                *restart_timer = true;
                let delay_ms = p
                    .restart_policy
                    .as_ref()
                    .map_or(1000, |rp| rp.restart_delay_ms);
                let tx = self.self_tx.clone();
                let generation = p.generation;
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    let _ = tx.send(PmCommand::AutoRestart { name, generation }).await;
                });
            }
        }
    }

    async fn handle_command(&mut self, cmd: PmCommand) {
        match cmd {
            PmCommand::Spawn {
                command,
                name,
                cwd,
                env,
                port,
                restart,
                watch,
                reply,
            } => {
                if self.stop_all.is_some() {
                    let _ = reply.send(stop_all_in_progress());
                    return;
                }
                let mut resp =
                    self.pm
                        .spawn_process(&command, name, cwd.as_deref(), env.as_ref(), port);
                // Attach restart/watch config if spawn succeeded
                let has_watch = watch.is_some();
                if let Response::RunOk { ref name, .. } = resp
                    && let Some(p) = self.pm.find_mut(name)
                {
                    p.restart_policy = restart;
                    p.watch_config = watch;
                }
                // Set up file watcher if watch config present
                if has_watch
                    && let Response::RunOk { ref name, .. } = resp
                    && let Err(e) = self.setup_watcher(name)
                {
                    // Process started but watcher failed — annotate the log.
                    // setup_watcher already cleared watch_config so status won't
                    // falsely report the process as watched.
                    let msg = format!("[agent-procs] Watch setup failed: {}", e);
                    if let Some(tx) = self.pm.find(name).and_then(|p| p.supervisor_tx()) {
                        let _ = tx.try_send(msg);
                    }
                }
                if let Response::RunOk {
                    ref name,
                    ref mut url,
                    port: Some(p),
                    ..
                } = resp
                    && let Some(pp) = self.proxy_port
                {
                    *url = Some(protocol::process_url(name, p, Some(pp)));
                }
                self.publish_proxy_state();
                let _ = reply.send(resp);
            }
            PmCommand::Stop { target, reply } => {
                self.pm.refresh_exit_states();
                let Some(p) = self.pm.find_mut(&target) else {
                    let _ = reply.send(not_found(&target));
                    return;
                };
                let name = p.name.clone();
                match &mut p.life {
                    Life::Stopping { then, waiters, .. } => {
                        *then = AfterStop::Park;
                        waiters.push(Waiter::Stop(reply));
                    }
                    Life::Exited { inc: None, .. } => {
                        p.watch_handle = None;
                        let _ = reply.send(stopped(&name));
                    }
                    Life::Running(_) | Life::Exited { inc: Some(_), .. } => {
                        self.start_teardown(&name, AfterStop::Park, Waiter::Stop(reply));
                    }
                }
            }
            PmCommand::StopAll { reply } => self.handle_stop_all(reply),
            PmCommand::Restart { target, reply } => {
                if self.stop_all.is_some() {
                    let _ = reply.send(stop_all_in_progress());
                    return;
                }
                self.pm.refresh_exit_states();
                let Some(p) = self.pm.find_mut(&target) else {
                    let _ = reply.send(not_found(&target));
                    return;
                };
                let name = p.name.clone();
                if let Life::Stopping { then, waiters, .. } = &mut p.life {
                    *then = AfterStop::Respawn(RespawnReason::Manual);
                    waiters.push(Waiter::Restart(reply));
                } else {
                    self.start_teardown(
                        &name,
                        AfterStop::Respawn(RespawnReason::Manual),
                        Waiter::Restart(reply),
                    );
                }
            }
            PmCommand::Status { reply } | PmCommand::StatusSnapshot { reply } => {
                let _ = reply.send(self.build_status());
            }
            PmCommand::HasProcess { target, reply } => {
                let _ = reply.send(self.pm.has_process(&target));
            }
            PmCommand::SessionName { reply } => {
                let _ = reply.send(self.pm.session_name().to_string());
            }
            PmCommand::IsProcessExited { target, reply } => {
                let exited = self.pm.is_process_exited(&target);
                // Checking may have observed an exit. The periodic refresh only
                // publishes when it sees one itself, so publish here or the
                // proxy keeps routing to the dead process.
                self.publish_proxy_state();
                let _ = reply.send(exited);
            }
            PmCommand::EnableProxy { proxy_port, reply } => {
                if let Some(existing) = self.proxy_port {
                    let _ = reply.send(Some(existing));
                } else {
                    self.proxy_port = Some(proxy_port);
                    self.pm.enable_proxy();
                    self.publish_proxy_state();
                    let _ = reply.send(None);
                }
            }
            PmCommand::Subscribe { reply } => {
                let _ = reply.send(self.pm.output_tx.subscribe());
            }
            PmCommand::AutoRestart { name, generation } => {
                if self.stop_all.is_some() {
                    return;
                }
                if let Some(p) = self.pm.find_mut(&name)
                    && p.generation == generation
                    && matches!(
                        p.life,
                        Life::Exited {
                            why: ExitWhy::Natural,
                            restart_timer: true,
                            ..
                        }
                    )
                {
                    p.restart_count += 1;
                    self.start_teardown(&name, AfterStop::Respawn(RespawnReason::Auto), None);
                }
            }
            PmCommand::WatchRestart { name, generation } => {
                if self.stop_all.is_some() {
                    return;
                }
                if let Some(p) = self.pm.find_mut(&name)
                    && p.generation == generation
                    && !matches!(
                        p.life,
                        Life::Stopping { .. }
                            | Life::Exited {
                                why: ExitWhy::Manual,
                                ..
                            }
                    )
                {
                    // Watch restarts are intentional, so they reset the crash budget.
                    p.restart_count = 0;
                    self.start_teardown(&name, AfterStop::Respawn(RespawnReason::Watch), None);
                }
            }
            PmCommand::StopFinished {
                name,
                generation,
                code,
            } => self.finish_teardown(&name, generation, code).await,
        }
    }

    /// Hand the live incarnation of `name` to a teardown task so the actor
    /// keeps serving other commands for the length of the grace period.
    fn start_teardown(&mut self, name: &str, then: AfterStop, waiter: impl Into<Option<Waiter>>) {
        let Some(p) = self.pm.find_mut(name) else {
            return;
        };
        p.watch_handle = None;
        let generation = p.generation;
        let waiters: Vec<Waiter> = waiter.into().into_iter().collect();
        let (inc, leader_exit) = match std::mem::replace(&mut p.life, Life::vacant()) {
            Life::Running(inc) => (Some(inc), None),
            Life::Exited { code, inc, .. } => (inc, Some(code)),
            Life::Stopping { .. } => unreachable!("callers join an in-flight stop instead"),
        };
        p.life = Life::Stopping {
            leader_exit,
            then,
            waiters,
        };
        let tx = self.self_tx.clone();
        let name = name.to_string();
        tokio::spawn(async move {
            let code = match inc {
                // A panicking teardown still reports back, so waiters never hang.
                Some(inc) => tokio::spawn(process_manager::teardown(inc, leader_exit))
                    .await
                    .unwrap_or(None),
                None => leader_exit.flatten(),
            };
            let _ = tx
                .send(PmCommand::StopFinished {
                    name,
                    generation,
                    code,
                })
                .await;
        });
    }

    async fn finish_teardown(&mut self, name: &str, generation: u64, code: Option<i32>) {
        let Some(p) = self.pm.find_mut(name) else {
            return;
        };
        if p.generation != generation {
            return;
        }
        let Life::Stopping { then, waiters, .. } = std::mem::replace(
            &mut p.life,
            Life::Exited {
                code,
                why: ExitWhy::Manual,
                inc: None,
                restart_timer: false,
            },
        ) else {
            unreachable!("only a Stopping record has a teardown in flight");
        };

        let restart_reply = match then {
            AfterStop::Park => actor_error("restart superseded by stop"),
            AfterStop::Respawn(reason) => self.respawn(name, reason, code).await,
        };
        for waiter in waiters {
            match waiter {
                Waiter::Stop(reply) => {
                    let _ = reply.send(stopped(name));
                }
                Waiter::Restart(reply) => {
                    let _ = reply.send(restart_reply.clone());
                }
            }
        }

        if let Some(batch) = &mut self.stop_all {
            batch.remaining.remove(name);
            if batch.remaining.is_empty() {
                self.finish_stop_all();
            }
        }
        self.publish_proxy_state();
    }

    async fn respawn(
        &mut self,
        name: &str,
        reason: RespawnReason,
        prev_code: Option<i32>,
    ) -> Response {
        if reason == RespawnReason::Manual
            && let Some(p) = self.pm.find_mut(name)
        {
            p.restart_count = 0;
        }
        let resp = self.pm.respawn(name, reason != RespawnReason::Manual).await;
        match (&resp, reason) {
            (Response::RunOk { .. }, _) => {
                let marker = match reason {
                    RespawnReason::Manual => None,
                    RespawnReason::Watch => {
                        Some("[agent-procs] File changed, restarted".to_string())
                    }
                    RespawnReason::Auto => self.pm.find(name).map(|p| {
                        let exit = prev_code.map_or("signal".into(), |c| c.to_string());
                        match p.restart_policy.as_ref().and_then(|rp| rp.max_restarts) {
                            Some(m) => format!(
                                "[agent-procs] Restarted (exit {}, attempt {}/{})",
                                exit, p.restart_count, m
                            ),
                            None => format!(
                                "[agent-procs] Restarted (exit {}, attempt {})",
                                exit, p.restart_count
                            ),
                        }
                    }),
                };
                if let Some(msg) = marker
                    && let Some(tx) = self.pm.find(name).and_then(|p| p.supervisor_tx())
                {
                    let _ = tx.try_send(msg);
                }
                let _ = self.setup_watcher(name);
            }
            (Response::Error { message, .. }, RespawnReason::Auto | RespawnReason::Watch) => {
                let label = if reason == RespawnReason::Auto {
                    "Restart"
                } else {
                    "Watch restart"
                };
                let _ = self.pm.output_tx.send(OutputLine {
                    process: name.to_string(),
                    stream: ProtoStream::Stdout,
                    line: format!("[agent-procs] {} failed: {}", label, message),
                });
            }
            _ => {}
        }
        resp
    }

    fn handle_stop_all(&mut self, reply: oneshot::Sender<Response>) {
        if let Some(batch) = &mut self.stop_all {
            batch.replies.push(reply);
            return;
        }
        self.pm.refresh_exit_states();
        let mut remaining = HashSet::new();
        for name in self.pm.names() {
            let Some(p) = self.pm.find_mut(&name) else {
                continue;
            };
            match &mut p.life {
                Life::Stopping { then, .. } => {
                    *then = AfterStop::Park;
                    remaining.insert(name);
                }
                Life::Exited { inc: None, .. } => {}
                Life::Running(_) | Life::Exited { inc: Some(_), .. } => {
                    self.start_teardown(&name, AfterStop::Park, None);
                    remaining.insert(name);
                }
            }
        }
        self.stop_all = Some(StopAllBatch {
            remaining,
            replies: vec![reply],
        });
        if self
            .stop_all
            .as_ref()
            .is_some_and(|b| b.remaining.is_empty())
        {
            self.finish_stop_all();
        }
        self.publish_proxy_state();
    }

    fn finish_stop_all(&mut self) {
        let Some(batch) = self.stop_all.take() else {
            return;
        };
        // Run and Restart were refused while the batch ran, so every record
        // left is one this stop-all covered.
        self.pm.clear();
        for reply in batch.replies {
            let _ = reply.send(Response::Ok {
                message: "all processes stopped".into(),
            });
        }
    }

    /// Set up a file watcher for a process if it has a `WatchConfig`.
    /// Returns `Err` with an error message if the watcher could not be created.
    /// On failure, clears `watch_config` so status/TUI don't report a phantom watcher.
    fn setup_watcher(&mut self, name: &str) -> Result<(), String> {
        let (paths, ignore, cwd) = {
            let Some(p) = self.pm.find(name) else {
                return Ok(());
            };
            let Some(ref wc) = p.watch_config else {
                return Ok(());
            };
            (
                wc.paths.clone(),
                wc.ignore.clone(),
                p.cwd.clone().unwrap_or_else(|| ".".to_string()),
            )
        };

        let base_dir = std::path::PathBuf::from(&cwd);
        let ignore_refs: Option<Vec<String>> = ignore;
        let ignore_slice = ignore_refs.as_deref();

        let tx = self.self_tx.clone();
        let proc_name = name.to_string();
        let generation = self.pm.find(name).map_or(0, |p| p.generation);
        let (restart_tx, mut restart_rx) = tokio::sync::mpsc::channel::<String>(16);

        // Forward watch events to PmCommand::WatchRestart
        tokio::spawn(async move {
            while let Some(name) = restart_rx.recv().await {
                let _ = tx.send(PmCommand::WatchRestart { name, generation }).await;
            }
        });

        match crate::daemon::watcher::create_watcher(
            &paths,
            ignore_slice,
            &base_dir,
            proc_name,
            restart_tx,
        ) {
            Ok(handle) => {
                if let Some(p) = self.pm.find_mut(name) {
                    p.watch_handle = Some(handle);
                }
                Ok(())
            }
            Err(e) => {
                tracing::warn!(process = %name, error = %e, "failed to create file watcher");
                // Clear watch_config so status/TUI don't report as watched
                if let Some(p) = self.pm.find_mut(name) {
                    p.watch_config = None;
                }
                Err(e)
            }
        }
    }

    /// Build a status response with proxy URL rewriting applied.
    fn build_status(&mut self) -> Response {
        if self.pm.refresh_exit_states() {
            self.publish_proxy_state();
        }
        let mut resp = self.pm.status_snapshot();
        self.rewrite_urls(&mut resp);
        resp
    }

    /// Rewrite process URLs to proxy form when proxy is enabled.
    fn rewrite_urls(&self, resp: &mut Response) {
        let Some(pp) = self.proxy_port else { return };
        if let Response::Status { ref mut processes } = *resp {
            for p in processes.iter_mut() {
                if let Some(port) = p.port {
                    p.url = Some(protocol::process_url(&p.name, port, Some(pp)));
                }
            }
        }
    }

    /// Publish current port map to the watch channel for lock-free proxy reads.
    /// Skips the update if the port map hasn't changed.
    fn publish_proxy_state(&self) {
        let new_map = self.pm.running_ports();
        let current = self.proxy_state_tx.borrow();
        if current.port_map == new_map {
            return;
        }
        drop(current);
        let _ = self.proxy_state_tx.send(ProxyState { port_map: new_map });
    }
}
