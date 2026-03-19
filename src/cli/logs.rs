use crate::cli::log_search::{SearchParams, SearchResult};
use crate::disk_log_reader::DiskLogReader;
use crate::paths;
use crate::protocol::{Request, Response};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
pub async fn execute(
    session: &str,
    target: Option<&str>,
    tail: usize,
    follow: bool,
    stderr: bool,
    all: bool,
    timeout: Option<u64>,
    lines: Option<usize>,
    grep: Option<String>,
    regex: bool,
    since: Option<String>,
    context: Option<u32>,
    json: bool,
) -> i32 {
    // Parse-time validation
    if context.is_some() && grep.is_none() {
        eprintln!("error: --context requires --grep");
        return 1;
    }
    if regex && grep.is_none() {
        eprintln!("error: --regex requires --grep");
        return 1;
    }

    if follow {
        if since.is_some() {
            eprintln!("warning: --since is ignored in follow mode");
        }
        if context.is_some() {
            eprintln!("warning: --context is ignored in follow mode");
        }
    }

    if follow {
        return execute_follow(
            session,
            target,
            tail,
            stderr,
            all,
            timeout,
            lines,
            grep.as_deref(),
            regex,
            json,
        )
        .await;
    }

    // Non-follow: use search pipeline
    let params = SearchParams {
        grep,
        regex,
        since,
        tail,
        context,
        stderr,
    };

    if all || target.is_none() {
        return search_all_processes(session, &params, json).await;
    }

    let target = target.unwrap();
    let log_dir = paths::log_dir(session);
    let mut reader = DiskLogReader::new(log_dir, target.to_string());

    let uptime_secs = if params.since.is_some() {
        get_process_uptime(session, target).await
    } else {
        None
    };

    match crate::cli::log_search::search_process(&mut reader, target, &params, uptime_secs) {
        Ok(result) => {
            crate::cli::log_search::print_results(&[result], json, context);
            0
        }
        Err(e) => {
            eprintln!("error: {}", e);
            1
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
async fn execute_follow(
    session: &str,
    target: Option<&str>,
    tail: usize,
    stderr: bool,
    all: bool,
    timeout: Option<u64>,
    lines: Option<usize>,
    grep: Option<&str>,
    regex: bool,
    json: bool,
) -> i32 {
    if let Some(code) = replay_follow_tail(session, target, tail, stderr, all, grep, regex, json) {
        return code;
    }

    let req = Request::Logs {
        target: target.map(std::string::ToString::to_string),
        tail: 0,
        follow: true,
        stderr,
        all: all || target.is_none(),
        timeout_secs: timeout.or(Some(30)), // CLI default; TUI passes None for infinite
        lines,
        grep: grep.map(String::from),
        regex,
    };

    let show_prefix = all || target.is_none();
    match crate::cli::stream_responses(session, &req, false, |process, stream, line| {
        if json {
            let stream_str = match stream {
                crate::protocol::Stream::Stdout => "stdout",
                crate::protocol::Stream::Stderr => "stderr",
            };
            let jl = serde_json::json!({
                "process": process,
                "stream": stream_str,
                "text": line,
            });
            println!("{}", jl);
        } else if show_prefix {
            println!("[{}] {}", process, line);
        } else {
            println!("{}", line);
        }
    })
    .await
    {
        Ok(Response::Error { code, message }) => {
            eprintln!("error: {}", message);
            code.exit_code()
        }
        Ok(_) => 0,
        Err(e) => {
            eprintln!("error: {}", e);
            1
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn replay_follow_tail(
    session: &str,
    target: Option<&str>,
    tail: usize,
    stderr: bool,
    all: bool,
    grep: Option<&str>,
    regex: bool,
    json: bool,
) -> Option<i32> {
    if tail == 0 {
        return None;
    }

    let log_dir = paths::log_dir(session);
    let params = SearchParams {
        grep: grep.map(String::from),
        regex,
        since: None,
        tail,
        context: None, // context is ignored in follow mode
        stderr,
    };

    if all || target.is_none() {
        // Replay all processes
        let mut results = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&log_dir) {
            let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
            for entry in entries.flatten() {
                let fname = entry.file_name().to_string_lossy().to_string();
                if let Some(name) = fname.strip_suffix(".stdout") {
                    names.insert(name.to_string());
                } else if let Some(name) = fname.strip_suffix(".stderr") {
                    names.insert(name.to_string());
                }
            }
            for name in &names {
                let mut reader = DiskLogReader::new(log_dir.clone(), name.clone());
                match crate::cli::log_search::search_process(&mut reader, name, &params, None) {
                    Ok(result) if !result.lines.is_empty() => results.push(result),
                    _ => {}
                }
            }
        }
        if !results.is_empty() {
            crate::cli::log_search::print_results(&results, json, None);
        }
        return None;
    }

    let target = target.expect("target should exist when not following all logs");
    let mut reader = DiskLogReader::new(log_dir, target.to_string());

    match crate::cli::log_search::search_process(&mut reader, target, &params, None) {
        Ok(result) => {
            if !result.lines.is_empty() {
                crate::cli::log_search::print_results(&[result], json, None);
            }
            None
        }
        Err(_) => None, // silently skip errors for tail replay
    }
}

async fn search_all_processes(session: &str, params: &SearchParams, json: bool) -> i32 {
    let log_dir = paths::log_dir(session);
    let uptimes = if params.since.is_some() {
        get_all_process_uptimes(session).await
    } else {
        HashMap::new()
    };

    let mut results: Vec<SearchResult> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&log_dir) {
        let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(name) = fname.strip_suffix(".stdout") {
                names.insert(name.to_string());
            } else if let Some(name) = fname.strip_suffix(".stderr") {
                names.insert(name.to_string());
            }
        }
        for name in &names {
            let mut reader = DiskLogReader::new(log_dir.clone(), name.clone());
            let uptime = uptimes.get(name.as_str()).copied().flatten();
            match crate::cli::log_search::search_process(&mut reader, name, params, uptime) {
                Ok(result) if !result.lines.is_empty() => results.push(result),
                _ => {}
            }
        }
    }

    crate::cli::log_search::print_results(&results, json, params.context);
    0
}

/// Query the daemon for the uptime of a specific process.
async fn get_process_uptime(session: &str, target: &str) -> Option<u64> {
    let req = Request::Status;
    let result = crate::cli::request(session, &req, false).await;
    match result {
        Ok(Response::Status { processes }) => {
            for p in processes {
                if p.name == target {
                    return p.uptime_secs;
                }
            }
            None
        }
        _ => None,
    }
}

/// Query the daemon for uptime of all processes.
async fn get_all_process_uptimes(session: &str) -> HashMap<String, Option<u64>> {
    let req = Request::Status;
    let result = crate::cli::request(session, &req, false).await;
    match result {
        Ok(Response::Status { processes }) => processes
            .into_iter()
            .map(|p| (p.name, p.uptime_secs))
            .collect(),
        _ => HashMap::new(),
    }
}

/// Read the last N lines from a file using a ring buffer.
///
/// Used by the TUI for initial log population; kept public(crate) for that use.
pub(crate) fn tail_file(path: &std::path::Path, n: usize) -> std::io::Result<Vec<String>> {
    if n == 0 {
        return Ok(Vec::new());
    }

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
