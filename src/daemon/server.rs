use crate::daemon::wait_engine;
use crate::protocol::{Request, Response};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::Mutex;

use super::process_manager::ProcessManager;

pub struct DaemonState {
    pub process_manager: ProcessManager,
    pub session: String,
}

pub async fn run(session: &str, socket_path: &Path) {
    let state = Arc::new(Mutex::new(DaemonState {
        process_manager: ProcessManager::new(session),
        session: session.to_string(),
    }));

    let listener = UnixListener::bind(socket_path).expect("failed to bind socket");

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(_) => break,
        };

        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let (reader, writer) = stream.into_split();
            let writer = Arc::new(Mutex::new(writer));
            let mut lines = BufReader::new(reader).lines();

            while let Ok(Some(line)) = lines.next_line().await {
                let request: Request = match serde_json::from_str(&line) {
                    Ok(r) => r,
                    Err(e) => {
                        let resp = Response::Error { code: 1, message: format!("invalid request: {}", e) };
                        let _ = send_response(&writer, &resp).await;
                        continue;
                    }
                };

                let is_shutdown = matches!(request, Request::Shutdown);

                let response = handle_request(&state, request).await;
                let _ = send_response(&writer, &response).await;

                if is_shutdown {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    std::process::exit(0);
                }
            }
        });
    }
}

async fn handle_request(state: &Arc<Mutex<DaemonState>>, request: Request) -> Response {
    match request {
        Request::Run { command, name, cwd } => {
            state.lock().await.process_manager.spawn_process(&command, name, cwd.as_deref()).await
        }
        Request::Stop { target } => {
            state.lock().await.process_manager.stop_process(&target).await
        }
        Request::StopAll => {
            state.lock().await.process_manager.stop_all().await
        }
        Request::Restart { target } => {
            state.lock().await.process_manager.restart_process(&target).await
        }
        Request::Status => {
            state.lock().await.process_manager.status()
        }
        Request::Wait { target, until, regex, exit, timeout_secs } => {
            // Check process exists
            {
                let s = state.lock().await;
                if !s.process_manager.has_process(&target) {
                    return Response::Error { code: 2, message: format!("process not found: {}", target) };
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
            ).await
        }
        Request::Logs { .. } => {
            // Logs are read directly from files by the CLI — no daemon involvement needed
            Response::Error { code: 1, message: "logs are read directly from disk by CLI".into() }
        }
        Request::Shutdown => {
            state.lock().await.process_manager.stop_all().await;
            Response::Ok { message: "daemon shutting down".into() }
        }
    }
}

async fn send_response(writer: &Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>, response: &Response) -> std::io::Result<()> {
    let mut w = writer.lock().await;
    let mut json = serde_json::to_string(response).unwrap();
    json.push('\n');
    w.write_all(json.as_bytes()).await?;
    w.flush().await
}
