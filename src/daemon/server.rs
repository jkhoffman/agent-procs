use crate::daemon::wait_engine;
use crate::protocol::{Request, Response};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::broadcast;
use tokio::sync::Mutex;

use super::process_manager::ProcessManager;

pub struct DaemonState {
    pub process_manager: ProcessManager,
    pub proxy_port: Option<u16>,
}

pub async fn run(session: &str, socket_path: &Path) {
    let state = Arc::new(Mutex::new(DaemonState {
        process_manager: ProcessManager::new(session),
        proxy_port: None,
    }));

    let listener = match UnixListener::bind(socket_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("fatal: failed to bind socket {:?}: {}", socket_path, e);
            return;
        }
    };

    // Shutdown signal: set to true when a Shutdown request is handled
    let shutdown = Arc::new(tokio::sync::Notify::new());

    loop {
        let (stream, _) = tokio::select! {
            result = listener.accept() => match result {
                Ok(conn) => conn,
                Err(e) => {
                    eprintln!("warning: accept error: {}", e);
                    continue;
                }
            },
            _ = shutdown.notified() => break,
        };

        let state = Arc::clone(&state);
        let shutdown = Arc::clone(&shutdown);
        tokio::spawn(async move {
            let (reader, writer) = stream.into_split();
            let writer = Arc::new(Mutex::new(writer));
            let mut lines = BufReader::new(reader).lines();

            while let Ok(Some(line)) = lines.next_line().await {
                let request: Request = match serde_json::from_str(&line) {
                    Ok(r) => r,
                    Err(e) => {
                        let resp = Response::Error {
                            code: 1,
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
                    let output_rx = state.lock().await.process_manager.output_tx.subscribe();
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

                let response = handle_request(&state, &shutdown, request).await;
                let _ = send_response(&writer, &response).await;

                if is_shutdown {
                    shutdown.notify_one();
                    return;
                }
            }
        });
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
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
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
    state: &Arc<Mutex<DaemonState>>,
    shutdown: &Arc<tokio::sync::Notify>,
    request: Request,
) -> Response {
    match request {
        Request::Run {
            command,
            name,
            cwd,
            env,
            port,
        } => {
            let mut s = state.lock().await;
            let proxy_port = s.proxy_port;
            let mut resp = s
                .process_manager
                .spawn_process(&command, name, cwd.as_deref(), env.as_ref(), port)
                .await;
            drop(s);
            if let Response::RunOk {
                ref name,
                ref mut url,
                port: Some(_),
                ..
            } = resp
            {
                if let Some(pp) = proxy_port {
                    *url = Some(format!("http://{}.localhost:{}", name, pp));
                }
            }
            resp
        }
        Request::Stop { target } => {
            state
                .lock()
                .await
                .process_manager
                .stop_process(&target)
                .await
        }
        Request::StopAll => state.lock().await.process_manager.stop_all().await,
        Request::Restart { target } => {
            state
                .lock()
                .await
                .process_manager
                .restart_process(&target)
                .await
        }
        Request::Status => {
            let mut s = state.lock().await;
            let proxy_port = s.proxy_port;
            let mut resp = s.process_manager.status();
            if let Some(pp) = proxy_port {
                if let Response::Status { ref mut processes } = resp {
                    for p in processes.iter_mut() {
                        if p.port.is_some() {
                            p.url = Some(format!("http://{}.localhost:{}", p.name, pp));
                        }
                    }
                }
            }
            resp
        }
        Request::Wait {
            target,
            until,
            regex,
            exit,
            timeout_secs,
        } => {
            // Check process exists
            {
                let s = state.lock().await;
                if !s.process_manager.has_process(&target) {
                    return Response::Error {
                        code: 2,
                        message: format!("process not found: {}", target),
                    };
                }
            }
            // Subscribe to output and delegate to wait engine
            let output_rx = state.lock().await.process_manager.output_tx.subscribe();
            let timeout = Duration::from_secs(timeout_secs.unwrap_or(30));
            let state_clone = Arc::clone(state);
            let target_clone = target.clone();
            wait_engine::wait_for(
                output_rx,
                &target,
                until.as_deref(),
                regex,
                exit,
                timeout,
                move || {
                    // This is called synchronously from the wait loop
                    // We can't hold the lock across the whole wait, so we check briefly
                    let state = state_clone.clone();
                    let target = target_clone.clone();
                    // Use try_lock to avoid deadlock
                    let result = match state.try_lock() {
                        Ok(mut s) => s.process_manager.is_process_exited(&target),
                        Err(_) => None,
                    };
                    result
                },
            )
            .await
        }
        Request::Logs { follow: false, .. } => {
            // Non-follow logs are read directly from files by the CLI — no daemon involvement needed
            Response::Error {
                code: 1,
                message: "non-follow logs are read directly from disk by CLI".into(),
            }
        }
        Request::Logs { follow: true, .. } => {
            // Handled separately in connection loop (needs streaming)
            Response::Error {
                code: 1,
                message: "follow requests handled in connection loop".into(),
            }
        }
        Request::Shutdown => {
            state.lock().await.process_manager.stop_all().await;
            Response::Ok {
                message: "daemon shutting down".into(),
            }
        }
        Request::EnableProxy { proxy_port } => {
            let mut s = state.lock().await;
            if let Some(existing_port) = s.proxy_port {
                return Response::Ok {
                    message: format!(
                        "Proxy already listening on http://localhost:{}",
                        existing_port
                    ),
                };
            }

            let (listener, port) = match super::proxy::bind_proxy_port(proxy_port) {
                Ok(pair) => pair,
                Err(e) => {
                    return Response::Error {
                        code: 1,
                        message: e,
                    }
                }
            };

            s.proxy_port = Some(port);
            s.process_manager.enable_proxy();
            drop(s);

            let proxy_state = Arc::clone(state);
            let proxy_shutdown = Arc::clone(shutdown);
            tokio::spawn(async move {
                if let Err(e) =
                    super::proxy::start_proxy(listener, port, proxy_state, proxy_shutdown).await
                {
                    eprintln!("proxy error: {}", e);
                }
            });

            Response::Ok {
                message: format!("Proxy listening on http://localhost:{}", port),
            }
        }
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
