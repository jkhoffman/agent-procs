use crate::protocol::{ProcessState, Request, Response};
use std::fmt::Write;

pub async fn execute(session: &str, json: bool) -> i32 {
    let req = Request::Status;
    crate::cli::request_and_handle(session, &req, false, |resp| match resp {
        Response::Status { processes } => {
            if json {
                match serde_json::to_string_pretty(&processes) {
                    Ok(json) => println!("{}", json),
                    Err(e) => {
                        eprintln!("error: failed to serialize status: {}", e);
                        return Some(1);
                    }
                }
            } else {
                let has_urls = processes.iter().any(|p| p.url.is_some());
                let has_restarts = processes
                    .iter()
                    .any(|p| p.restart_policy.is_some() || p.restart_count.is_some());
                let has_watch = processes.iter().any(|p| p.watched == Some(true));

                // Build header
                let mut header =
                    format!("{:<12} {:<8} {:<10} {:<6}", "NAME", "PID", "STATE", "EXIT");
                if has_restarts {
                    let _ = write!(header, " {:<10}", "RESTARTS");
                }
                if has_watch {
                    let _ = write!(header, " {:<6}", "WATCH");
                }
                if has_urls {
                    let _ = write!(header, " {:<30}", "URL");
                }
                let _ = write!(header, " UPTIME");
                println!("{}", header);

                for p in &processes {
                    let state = match p.state {
                        ProcessState::Running => "running",
                        ProcessState::Exited => "exited",
                        ProcessState::Failed => "FAILED",
                        ProcessState::Unknown => "unknown",
                    };
                    let exit = p.exit_code.map_or("-".into(), |c| c.to_string());
                    let uptime = p.uptime_secs.map_or("-".into(), format_uptime);

                    let mut line = format!("{:<12} {:<8} {:<10} {:<6}", p.name, p.pid, state, exit);

                    if has_restarts {
                        let restarts = match (p.restart_count, p.max_restarts) {
                            (Some(c), Some(m)) => format!("{}/{}", c, m),
                            (Some(c), None) => c.to_string(),
                            _ => "-".into(),
                        };
                        let _ = write!(line, " {:<10}", restarts);
                    }
                    if has_watch {
                        let watch = if p.watched == Some(true) { "*" } else { "-" };
                        let _ = write!(line, " {:<6}", watch);
                    }
                    if has_urls {
                        let url = p.url.as_deref().unwrap_or("-");
                        let _ = write!(line, " {:<30}", url);
                    }
                    let _ = write!(line, " {}", uptime);
                    println!("{}", line);
                }
            }
            Some(0)
        }
        _ => None,
    })
    .await
}

fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}
