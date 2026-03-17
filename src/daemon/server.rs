use crate::daemon::actor::{PmHandle, ProcessManagerActor, ProxyState};
use crate::daemon::wait_engine;
use crate::protocol::{self, ErrorCode, PROTOCOL_VERSION, Request, Response};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::Mutex;
use tokio::sync::{broadcast, watch};

/// Maximum concurrent client connections.  Prevents accidental fork-bomb
/// loops where each connection spawns more connections.
const MAX_CONCURRENT_CONNECTIONS: usize = 64;

pub async fn run(session: &str, socket_path: &Path) {
    let (handle, proxy_state_rx, actor) = ProcessManagerActor::new(session);

    // Spawn the actor loop
    tokio::spawn(actor.run());

    let listener = match UnixListener::bind(socket_path) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(path = %socket_path.display(), error = %e, "failed to bind socket");
            return;
        }
    };

    // Shutdown signal: set to true when a Shutdown request is handled
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let active_connections = Arc::new(AtomicUsize::new(0));

    loop {
        let (stream, _) = tokio::select! {
            result = listener.accept() => match result {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::warn!(error = %e, "accept error");
                    continue;
                }
            },
            () = shutdown.notified() => break,
        };

        // Rate limiting: atomically increment then check to avoid TOCTOU race
        let prev = active_connections.fetch_add(1, Ordering::AcqRel);
        if prev >= MAX_CONCURRENT_CONNECTIONS {
            active_connections.fetch_sub(1, Ordering::AcqRel);
            tracing::warn!(
                current = prev,
                max = MAX_CONCURRENT_CONNECTIONS,
                "connection rejected: too many concurrent connections"
            );
            drop(stream);
            continue;
        }

        let handle = handle.clone();
        let shutdown = Arc::clone(&shutdown);
        let conn_counter = Arc::clone(&active_connections);
        let proxy_state_rx = proxy_state_rx.clone();
        tokio::spawn(async move {
            let _guard = ConnectionGuard(conn_counter);
            let (reader, writer) = stream.into_split();
            let writer = Arc::new(Mutex::new(writer));
            // Wrap reader in a size-limited adapter so read_line cannot
            // allocate more than MAX_MESSAGE_SIZE bytes.
            let limited = reader.take(protocol::MAX_MESSAGE_SIZE as u64);
            let mut reader = BufReader::new(limited);

            loop {
                let mut line = String::new();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break, // EOF or error
                    Ok(n) if n >= protocol::MAX_MESSAGE_SIZE => {
                        let resp = Response::Error {
                            code: ErrorCode::General,
                            message: format!(
                                "message too large: {} bytes (max {})",
                                n,
                                protocol::MAX_MESSAGE_SIZE
                            ),
                        };
                        let _ = send_response(&writer, &resp).await;
                        break; // disconnect oversized clients
                    }
                    Ok(_) => {}
                }
                // Reset the take limit for the next message
                reader
                    .get_mut()
                    .set_limit(protocol::MAX_MESSAGE_SIZE as u64);

                let request: Request = match serde_json::from_str(&line) {
                    Ok(r) => r,
                    Err(e) => {
                        let resp = Response::Error {
                            code: ErrorCode::General,
                            message: format!("invalid request: {}", e),
                        };
                        let _ = send_response(&writer, &resp).await;
                        continue;
                    }
                };

                // Handle follow requests with streaming (before handle_request)
                if let Request::Logs {
                    follow: true,
                    ref target,
                    all,
                    timeout_secs,
                    lines,
                    ..
                } = request
                {
                    let output_rx = handle.subscribe().await;
                    let max_lines = lines;
                    let target_filter = target.clone();
                    let show_all = all;

                    handle_follow_stream(
                        &writer,
                        output_rx,
                        target_filter,
                        show_all,
                        timeout_secs,
                        max_lines,
                    )
                    .await;
                    continue; // Don't call handle_request
                }

                let is_shutdown = matches!(request, Request::Shutdown);

                let response = handle_request(&handle, &shutdown, &proxy_state_rx, request).await;
                let _ = send_response(&writer, &response).await;

                if is_shutdown {
                    shutdown.notify_one();
                    return;
                }
            }
        });
    }
}

/// RAII guard that decrements the active connection counter when dropped.
struct ConnectionGuard(Arc<AtomicUsize>);

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn handle_follow_stream(
    writer: &Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
    mut output_rx: broadcast::Receiver<super::log_writer::OutputLine>,
    target: Option<String>,
    all: bool,
    timeout_secs: Option<u64>,
    max_lines: Option<usize>,
) {
    let mut line_count: usize = 0;

    let stream_loop = async {
        loop {
            match output_rx.recv().await {
                Ok(output_line) => {
                    if !all {
                        if let Some(ref t) = target {
                            if output_line.process != *t {
                                continue;
                            }
                        }
                    }

                    let resp = Response::LogLine {
                        process: output_line.process,
                        stream: output_line.stream,
                        line: output_line.line,
                    };
                    if send_response(writer, &resp).await.is_err() {
                        return;
                    }

                    line_count += 1;
                    if let Some(max) = max_lines {
                        if line_count >= max {
                            return;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    };

    // Apply timeout only if specified; otherwise stream indefinitely
    match timeout_secs {
        Some(secs) => {
            let _ = tokio::time::timeout(Duration::from_secs(secs), stream_loop).await;
        }
        None => {
            stream_loop.await;
        }
    }

    let _ = send_response(writer, &Response::LogEnd).await;
}

async fn handle_request(
    handle: &PmHandle,
    shutdown: &Arc<tokio::sync::Notify>,
    proxy_state_rx: &watch::Receiver<ProxyState>,
    request: Request,
) -> Response {
    match request {
        Request::Run {
            command,
            name,
            cwd,
            env,
            port,
        } => handle.spawn_process(command, name, cwd, env, port).await,
        Request::Stop { target } => handle.stop_process(&target).await,
        Request::StopAll => handle.stop_all().await,
        Request::Restart { target } => handle.restart_process(&target).await,
        Request::Status => handle.status().await,
        Request::Wait {
            target,
            until,
            regex,
            exit,
            timeout_secs,
        } => {
            // Check process exists
            if !handle.has_process(&target).await {
                return Response::Error {
                    code: ErrorCode::NotFound,
                    message: format!("process not found: {}", target),
                };
            }

            let session_name = handle.session_name().await;

            // Subscribe BEFORE checking historical logs to avoid missing lines
            let output_rx = handle.subscribe().await;

            // Check historical log output for the pattern
            if let Some(ref pattern) = until {
                let log_path =
                    crate::paths::log_dir(&session_name).join(format!("{}.stdout", target));
                if let Ok(content) = std::fs::read_to_string(&log_path) {
                    let compiled_re = if regex {
                        regex::Regex::new(pattern).ok()
                    } else {
                        None
                    };
                    let matched_line = content.lines().find(|line| {
                        if let Some(ref re) = compiled_re {
                            re.is_match(line)
                        } else {
                            line.contains(pattern.as_str())
                        }
                    });
                    if let Some(line) = matched_line {
                        return Response::WaitMatch {
                            line: line.to_string(),
                        };
                    }
                }
            }
            let timeout = Duration::from_secs(timeout_secs.unwrap_or(30));
            wait_engine::wait_for(
                output_rx,
                &target,
                until.as_deref(),
                regex,
                exit,
                timeout,
                handle.clone(),
            )
            .await
        }
        Request::Logs { follow: false, .. } => Response::Error {
            code: ErrorCode::General,
            message: "non-follow logs are read directly from disk by CLI".into(),
        },
        Request::Logs { follow: true, .. } => Response::Error {
            code: ErrorCode::General,
            message: "follow requests handled in connection loop".into(),
        },
        Request::Shutdown => {
            let _ = handle.stop_all().await;
            Response::Ok {
                message: "daemon shutting down".into(),
            }
        }
        Request::EnableProxy { proxy_port } => {
            let (listener, port) = match super::proxy::bind_proxy_port(proxy_port) {
                Ok(pair) => pair,
                Err(e) => {
                    return Response::Error {
                        code: ErrorCode::General,
                        message: e.to_string(),
                    };
                }
            };

            if let Some(existing) = handle.enable_proxy(port).await {
                return Response::Ok {
                    message: format!("Proxy already listening on http://localhost:{}", existing),
                };
            }

            let proxy_handle = handle.clone();
            let proxy_shutdown = Arc::clone(shutdown);
            let proxy_rx = proxy_state_rx.clone();
            tokio::spawn(async move {
                if let Err(e) = super::proxy::start_proxy(
                    listener,
                    port,
                    proxy_handle,
                    proxy_rx,
                    proxy_shutdown,
                )
                .await
                {
                    tracing::error!(error = %e, "proxy error");
                }
            });

            Response::Ok {
                message: format!("Proxy listening on http://localhost:{}", port),
            }
        }
        Request::Hello { .. } => Response::Hello {
            version: PROTOCOL_VERSION,
        },
        Request::Unknown => Response::Error {
            code: ErrorCode::General,
            message: "unknown request type".into(),
        },
    }
}

async fn send_response(
    writer: &Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
    response: &Response,
) -> std::io::Result<()> {
    let mut w = writer.lock().await;
    let mut json = serde_json::to_string(response)
        .expect("Response serialization should never fail for well-typed enums");
    json.push('\n');
    w.write_all(json.as_bytes()).await?;
    w.flush().await
}
