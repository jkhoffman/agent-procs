use crate::paths;
use crate::session;

pub async fn list() -> i32 {
    let base = paths::socket_base_dir();

    let entries = match std::fs::read_dir(&base) {
        Ok(e) => e,
        Err(_) => {
            println!("no active sessions");
            return 0;
        }
    };

    println!("{:<20} STATUS", "SESSION");
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".pid") {
            continue;
        }
        let session_name = name.trim_end_matches(".pid");
        let status = if session::is_daemon_alive(&entry.path()) {
            "running"
        } else {
            "stale"
        };
        println!("{:<20} {}", session_name, status);
    }
    0
}

pub async fn clean() -> i32 {
    let base = paths::socket_base_dir();

    let entries = match std::fs::read_dir(&base) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".pid") {
            continue;
        }
        let session_name = name.trim_end_matches(".pid");

        if !session::is_daemon_alive(&entry.path()) {
            // Remove socket and PID files
            let _ = std::fs::remove_file(paths::socket_path(session_name));
            let _ = std::fs::remove_file(paths::pid_path(session_name));
            // Remove XDG state directory (logs, state.json)
            let _ = std::fs::remove_dir_all(paths::state_dir(session_name));
            println!("cleaned stale session: {}", session_name);
        }
    }
    0
}
