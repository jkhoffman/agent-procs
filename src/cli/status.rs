use crate::protocol::{ProcessState, Request, Response};

pub async fn execute(session: &str, json: bool) -> i32 {
    let req = Request::Status;
    match crate::cli::request(session, &req, false).await {
        Ok(Response::Status { processes }) => {
            if json {
                match serde_json::to_string_pretty(&processes) {
                    Ok(json) => println!("{}", json),
                    Err(e) => {
                        eprintln!("error: failed to serialize status: {}", e);
                        return 1;
                    }
                }
            } else {
                let has_urls = processes.iter().any(|p| p.url.is_some());
                if has_urls {
                    println!(
                        "{:<12} {:<8} {:<10} {:<6} {:<30} UPTIME",
                        "NAME", "PID", "STATE", "EXIT", "URL"
                    );
                } else {
                    println!(
                        "{:<12} {:<8} {:<10} {:<6} UPTIME",
                        "NAME", "PID", "STATE", "EXIT"
                    );
                }
                for p in &processes {
                    let state = match p.state {
                        ProcessState::Running => "running",
                        ProcessState::Exited => "exited",
                    };
                    let exit = p.exit_code.map(|c| c.to_string()).unwrap_or("-".into());
                    let uptime = p.uptime_secs.map(format_uptime).unwrap_or("-".into());
                    if has_urls {
                        let url = p.url.as_deref().unwrap_or("-");
                        println!(
                            "{:<12} {:<8} {:<10} {:<6} {:<30} {}",
                            p.name, p.pid, state, exit, url, uptime
                        );
                    } else {
                        println!(
                            "{:<12} {:<8} {:<10} {:<6} {}",
                            p.name, p.pid, state, exit, uptime
                        );
                    }
                }
            }
            0
        }
        Ok(Response::Error { code, message }) => {
            eprintln!("error: {}", message);
            code
        }
        _ => 1,
    }
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
