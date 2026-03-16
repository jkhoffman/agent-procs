use crate::paths;
use crate::protocol::{Request, Response};
use std::fs::File;
use std::io::{BufRead, BufReader};

#[allow(clippy::too_many_arguments)]
pub async fn execute(
    session: &str,
    target: Option<&str>,
    tail: usize,
    follow: bool,
    stderr: bool,
    all: bool,
    timeout: Option<u64>,
    lines: Option<usize>,
) -> i32 {
    if follow {
        return execute_follow(session, target, all, timeout, lines).await;
    }

    // Non-follow: read from disk (unchanged)
    let log_dir = paths::log_dir(session);

    if all || target.is_none() {
        return show_all_logs(&log_dir, tail);
    }

    let target = target.unwrap();
    let stream = if stderr { "stderr" } else { "stdout" };
    let path = log_dir.join(format!("{}.{}", target, stream));

    match tail_file(&path, tail) {
        Ok(lines) => {
            for line in lines {
                println!("{}", line);
            }
            0
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("error: no logs for process '{}' ({})", target, stream);
            2
        }
        Err(e) => {
            eprintln!("error reading logs: {}", e);
            1
        }
    }
}

async fn execute_follow(
    session: &str,
    target: Option<&str>,
    all: bool,
    timeout: Option<u64>,
    lines: Option<usize>,
) -> i32 {
    let req = Request::Logs {
        target: target.map(std::string::ToString::to_string),
        tail: 0,
        follow: true,
        stderr: false,
        all: all || target.is_none(),
        timeout_secs: timeout.or(Some(30)), // CLI default; TUI passes None for infinite
        lines,
    };

    let show_prefix = all || target.is_none();
    match crate::cli::stream_responses(session, &req, false, |process, _stream, line| {
        if show_prefix {
            println!("[{}] {}", process, line);
        } else {
            println!("{}", line);
        }
    })
    .await
    {
        Ok(Response::Error { code, message }) => {
            eprintln!("error: {}", message);
            code
        }
        Ok(_) => 0,
        Err(e) => {
            eprintln!("error: {}", e);
            1
        }
    }
}

fn show_all_logs(log_dir: &std::path::Path, tail: usize) -> i32 {
    let entries = match std::fs::read_dir(log_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: cannot read log dir: {}", e);
            return 1;
        }
    };

    let mut all_lines: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".stdout") {
            continue;
        }
        let proc_name = name.trim_end_matches(".stdout").to_string();
        if let Ok(lines) = tail_file(&entry.path(), tail) {
            for line in lines {
                all_lines.push((proc_name.clone(), line));
            }
        }
    }

    for (proc_name, line) in &all_lines {
        println!("[{}] {}", proc_name, line);
    }
    0
}

fn tail_file(path: &std::path::Path, n: usize) -> std::io::Result<Vec<String>> {
    let file = File::open(path)?;
    // Use a ring buffer to keep only the last N lines in memory
    let mut ring: std::collections::VecDeque<String> = std::collections::VecDeque::with_capacity(n);
    for line in BufReader::new(file).lines() {
        let line = line?;
        if ring.len() == n {
            ring.pop_front();
        }
        ring.push_back(line);
    }
    Ok(ring.into_iter().collect())
}
