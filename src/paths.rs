use std::env;
use std::path::PathBuf;

/// Base directory for sockets and PID files: /tmp/agent-procs-<uid>/
/// Short fixed path to avoid macOS 103-byte socket path limit.
pub fn socket_base_dir() -> PathBuf {
    let uid = nix::unistd::getuid();
    PathBuf::from(format!("/tmp/agent-procs-{}", uid))
}

pub fn socket_path(session: &str) -> PathBuf {
    socket_base_dir().join(format!("{}.sock", session))
}

pub fn pid_path(session: &str) -> PathBuf {
    socket_base_dir().join(format!("{}.pid", session))
}

/// State directory for persistent data (logs, state.json).
/// Uses $XDG_STATE_HOME, defaults to ~/.local/state/.
pub fn state_dir(session: &str) -> PathBuf {
    let base = match env::var("XDG_STATE_HOME") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => {
            let home = env::var("HOME").expect("HOME not set");
            PathBuf::from(home).join(".local/state")
        }
    };
    base.join("agent-procs/sessions").join(session)
}

pub fn log_dir(session: &str) -> PathBuf { state_dir(session).join("logs") }
pub fn state_file(session: &str) -> PathBuf { state_dir(session).join("state.json") }
