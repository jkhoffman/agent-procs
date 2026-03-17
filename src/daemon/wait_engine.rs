use crate::daemon::actor::PmHandle;
use crate::daemon::log_writer::OutputLine;
use crate::protocol::{ErrorCode, Response};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::{self, MissedTickBehavior};

const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

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
    if wait_exit && let Some(exit_code) = handle.is_process_exited(target).await {
        return Response::WaitExited { exit_code };
    }

    let mut exit_poll = time::interval(EXIT_POLL_INTERVAL);
    exit_poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
    exit_poll.tick().await;

    tokio::select! {
        result = async {
            loop {
                tokio::select! {
                    recv = output_rx.recv() => match recv {
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
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {},
                        Err(broadcast::error::RecvError::Closed) => {
                            if wait_exit
                                && let Some(exit_code) = handle.is_process_exited(target).await
                            {
                                return Response::WaitExited { exit_code };
                            }
                            return Response::Error {
                                code: ErrorCode::General,
                                message: "output channel closed".into(),
                            };
                        }
                    },
                    _ = exit_poll.tick(), if wait_exit => {
                        if let Some(exit_code) = handle.is_process_exited(target).await {
                            return Response::WaitExited { exit_code };
                        }
                    }
                }
            }
        } => result,
        () = tokio::time::sleep(timeout) => Response::WaitTimeout,
    }
}
