use crate::daemon::log_writer::OutputLine;
use crate::daemon::process_manager::ProcessManager;
use crate::protocol::{self, ErrorCode, Response};
use std::collections::HashMap;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

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
        let (reply, rx) = oneshot::channel();
        let _ = self
            .tx
            .send(PmCommand::Spawn {
                command,
                name,
                cwd,
                env,
                port,
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

        let handle = PmHandle { tx };
        let actor = Self {
            pm,
            rx,
            proxy_state_tx,
            proxy_port: None,
        };

        (handle, proxy_state_rx, actor)
    }

    /// Run the actor loop until all senders are dropped.
    pub async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            self.handle_command(cmd).await;
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
                reply,
            } => {
                let mut resp = self
                    .pm
                    .spawn_process(&command, name, cwd.as_deref(), env.as_ref(), port)
                    .await;
                if let Response::RunOk {
                    ref name,
                    ref mut url,
                    port: Some(p),
                    ..
                } = resp
                {
                    if let Some(pp) = self.proxy_port {
                        *url = Some(protocol::process_url(name, p, Some(pp)));
                    }
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
        }
    }

    /// Build a status response with proxy URL rewriting applied.
    fn build_status(&mut self) -> Response {
        let mut resp = self.pm.status();
        self.rewrite_urls(&mut resp);
        resp
    }

    /// Build a status snapshot with proxy URL rewriting applied.
    fn build_status_snapshot(&self) -> Response {
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
