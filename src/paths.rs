use std::env;
use std::path::PathBuf;

/// macOS limits Unix socket paths to 103 bytes; Linux allows 107.
const MAX_SOCKET_PATH_LEN: usize = 103;

/// Base directory for sockets and PID files: /tmp/agent-procs-<uid>/
/// Short fixed path to avoid macOS 103-byte socket path limit.
pub fn socket_base_dir() -> PathBuf {
    let uid = nix::unistd::getuid();
    PathBuf::from(format!("/tmp/agent-procs-{}", uid))
}

pub fn socket_path(session: &str) -> PathBuf {
    let path = socket_base_dir().join(format!("{}.sock", session));
    if path.as_os_str().len() > MAX_SOCKET_PATH_LEN {
        eprintln!(
            "warning: socket path exceeds {} bytes ({} bytes): {:?}. \
             Use a shorter session name.",
            MAX_SOCKET_PATH_LEN,
            path.as_os_str().len(),
            path
        );
    }
    path
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
            let home = env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home).join(".local/state")
        }
    };
    base.join("agent-procs/sessions").join(session)
}

pub fn log_dir(session: &str) -> PathBuf {
    state_dir(session).join("logs")
}
pub fn state_file(session: &str) -> PathBuf {
    state_dir(session).join("state.json")
}
