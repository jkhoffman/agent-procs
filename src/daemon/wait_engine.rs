use crate::daemon::actor::PmHandle;
use crate::daemon::log_writer::OutputLine;
use crate::protocol::{ErrorCode, Response};
use std::time::Duration;
use tokio::sync::broadcast;

/// Wait for a condition on a process's output.
/// Returns a Response indicating match, exit, or timeout.
pub async fn wait_for(
    mut output_rx: broadcast::Receiver<OutputLine>,
    target: &str,
    pattern: Option<&str>,
    use_regex: bool,
    wait_exit: bool,
    timeout: Duration,
    handle: PmHandle,
) -> Response {
    let compiled_regex = if use_regex {
        match pattern {
            Some(p) => match regex::Regex::new(p) {
                Ok(re) => Some(re),
                Err(e) => {
                    return Response::Error {
                        code: ErrorCode::General,
                        message: format!("invalid regex: {}", e),
                    };
                }
            },
            None => None,
        }
    } else {
        None
    };

    // Check if already exited before we start waiting
    if wait_exit {
        if let Some(exit_code) = handle.is_process_exited(target).await {
            return Response::WaitExited { exit_code };
        }
    }

    tokio::select! {
        result = async {
            loop {
                match output_rx.recv().await {
                    Ok(line) => {
                        if line.process != target { continue; }
                        if let Some(pat) = pattern {
                            let matched = if let Some(ref re) = compiled_regex {
                                re.is_match(&line.line)
                            } else {
                                line.line.contains(pat)
                            };
                            if matched {
                                return Response::WaitMatch { line: line.line };
                            }
                        }
                        // After each line, check if process exited (for --exit mode)
                        if wait_exit {
                            if let Some(exit_code) = handle.is_process_exited(target).await {
                                return Response::WaitExited { exit_code };
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {},
                    Err(broadcast::error::RecvError::Closed) => {
                        if wait_exit {
                            // Channel closed — process likely exited
                            if let Some(exit_code) = handle.is_process_exited(target).await {
                                return Response::WaitExited { exit_code };
                            }
                        }
                        return Response::Error { code: ErrorCode::General, message: "output channel closed".into() };
                    }
                }
            }
        } => result,
        () = tokio::time::sleep(timeout) => Response::WaitTimeout,
    }
}
