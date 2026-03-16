use crate::paths;
use crate::session;

/// Iterate PID files in the socket base dir, yielding `(session_name, entry)` pairs.
fn pid_entries() -> impl Iterator<Item = (String, std::fs::DirEntry)> {
    let base = paths::socket_base_dir();
    std::fs::read_dir(base)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_pid = std::path::Path::new(&name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pid"));
            if is_pid {
                let session_name = name.trim_end_matches(".pid").to_string();
                Some((session_name, entry))
            } else {
                None
            }
        })
}

pub fn list() -> i32 {
    let mut found = false;
    for (session_name, entry) in pid_entries() {
        if !found {
            println!("{:<20} STATUS", "SESSION");
            found = true;
        }
        let status = if session::is_daemon_alive(&entry.path()) {
            "running"
        } else {
            "stale"
        };
        println!("{:<20} {}", session_name, status);
    }
    if !found {
        println!("no active sessions");
    }
    0
}

pub fn clean() -> i32 {
    for (session_name, entry) in pid_entries() {
        if !session::is_daemon_alive(&entry.path()) {
            let _ = std::fs::remove_file(paths::socket_path(&session_name));
            let _ = std::fs::remove_file(paths::pid_path(&session_name));
            let _ = std::fs::remove_dir_all(paths::state_dir(&session_name));
            println!("cleaned stale session: {}", session_name);
        }
    }
    0
}
