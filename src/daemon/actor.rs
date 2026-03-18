use crate::daemon::log_writer::OutputLine;
use crate::daemon::process_manager::ProcessManager;
use crate::protocol::{self, ErrorCode, Response, Stream as ProtoStream};
use std::collections::HashMap;
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
    },
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
        // First, mark processes that have exhausted their restarts as failed
        let exhausted: Vec<String> = self.pm.exhausted_restart_processes().into_iter().collect();
        for name in &exhausted {
            if let Some(p) = self.pm.find(name)
                && let Some(ref policy) = p.restart_policy
                && let Some(max) = policy.max_restarts
                && let Some(ref tx) = p.supervisor_tx
            {
                let msg = format!("[agent-procs] Max restarts ({}) exhausted", max);
                let _ = tx.try_send(msg);
            }
            self.pm.mark_failed(name);
        }
        if !exhausted.is_empty() {
            self.publish_proxy_state();
        }

        // Then schedule restarts for eligible processes
        let restartable = self.pm.collect_restartable_processes();
        for name in restartable {
            if let Some(p) = self.pm.find_mut(&name) {
                p.restart_pending = true;
                let delay_ms = p
                    .restart_policy
                    .as_ref()
                    .map_or(1000, |rp| rp.restart_delay_ms);
                let tx = self.self_tx.clone();
                let name_clone = name.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    let _ = tx.send(PmCommand::AutoRestart { name: name_clone }).await;
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
                let mut resp = self
                    .pm
                    .spawn_process(&command, name, cwd.as_deref(), env.as_ref(), port)
                    .await;
                // Attach restart/watch config if spawn succeeded
                if let Response::RunOk { ref name, .. } = resp
                    && let Some(p) = self.pm.find_mut(name)
                {
                    p.restart_policy = restart;
                    p.watch_config = watch;
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
                let resp = self.pm.stop_process(&target).await;
                self.publish_proxy_state();
                let _ = reply.send(resp);
            }
            PmCommand::StopAll { reply } => {
                let resp = self.pm.stop_all().await;
                self.publish_proxy_state();
                let _ = reply.send(resp);
            }
            PmCommand::Restart { target, reply } => {
                let resp = self.pm.restart_process(&target).await;
                self.publish_proxy_state();
                let _ = reply.send(resp);
            }
            PmCommand::Status { reply } => {
                let _ = reply.send(self.build_status());
            }
            PmCommand::StatusSnapshot { reply } => {
                let _ = reply.send(self.build_status_snapshot());
            }
            PmCommand::HasProcess { target, reply } => {
                let _ = reply.send(self.pm.has_process(&target));
            }
            PmCommand::SessionName { reply } => {
                let _ = reply.send(self.pm.session_name().to_string());
            }
            PmCommand::IsProcessExited { target, reply } => {
                let _ = reply.send(self.pm.is_process_exited(&target));
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
            PmCommand::AutoRestart { name } => {
                self.handle_auto_restart(&name).await;
            }
        }
    }

    async fn handle_auto_restart(&mut self, name: &str) {
        // Re-check guards
        let should_restart = self
            .pm
            .find(name)
            .is_some_and(|p| p.child.is_none() && !p.manually_stopped && p.restart_pending);
        if !should_restart {
            if let Some(p) = self.pm.find_mut(name) {
                p.restart_pending = false;
            }
            return;
        }

        // Increment count
        if let Some(p) = self.pm.find_mut(name) {
            p.restart_count += 1;
        }

        // Respawn
        match self.pm.respawn_in_place(name).await {
            Ok(()) => {
                // Send success annotation to new capture task
                if let Some(p) = self.pm.find(name) {
                    let count = p.restart_count;
                    let max = p.restart_policy.as_ref().and_then(|rp| rp.max_restarts);
                    let exit = p.exit_code.map_or("signal".into(), |c: i32| c.to_string());
                    let msg = match max {
                        Some(m) => {
                            format!(
                                "[agent-procs] Restarted (exit {}, attempt {}/{})",
                                exit, count, m
                            )
                        }
                        None => {
                            format!("[agent-procs] Restarted (exit {}, attempt {})", exit, count)
                        }
                    };
                    if let Some(tx) = &p.supervisor_tx {
                        let _ = tx.send(msg).await;
                    }
                }
            }
            Err(err) => {
                // Broadcast failure (live only)
                let msg = format!("[agent-procs] Restart failed: {}", err);
                let _ = self.pm.output_tx.send(OutputLine {
                    process: name.to_string(),
                    stream: ProtoStream::Stdout,
                    line: msg,
                });
            }
        }

        // Clear pending
        if let Some(p) = self.pm.find_mut(name) {
            p.restart_pending = false;
        }
        self.publish_proxy_state();
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

    /// Build a status snapshot with proxy URL rewriting applied.
    fn build_status_snapshot(&mut self) -> Response {
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
