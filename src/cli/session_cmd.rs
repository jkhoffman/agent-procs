use crate::paths;
use crate::session;

pub async fn list() -> i32 {
    let runtime_base = paths::sessions_base_dir();

    let entries = match std::fs::read_dir(&runtime_base) {
        Ok(e) => e,
        Err(_) => {
            println!("no active sessions");
            return 0;
        }
    };

    println!("{:<20} STATUS", "SESSION");
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let pid_path = entry.path().join("daemon.pid");
        let status = if session::is_daemon_alive(&pid_path) {
            "running"
        } else {
            "stale"
        };
        println!("{:<20} {}", name, status);
    }
    0
}

pub async fn clean() -> i32 {
    let runtime_base = paths::sessions_base_dir();

    let entries = match std::fs::read_dir(&runtime_base) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let pid_path = entry.path().join("daemon.pid");
        if !session::is_daemon_alive(&pid_path) {
            let name = entry.file_name().to_string_lossy().to_string();
            let _ = std::fs::remove_dir_all(entry.path());
            let _ = std::fs::remove_dir_all(paths::state_dir(&name));
            println!("cleaned stale session: {}", name);
        }
    }
    0
}
