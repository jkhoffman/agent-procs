//! Socket, PID file, and log directory path resolution.
//!
//! Sockets and PID files live under `/tmp/agent-procs-<uid>/` to stay
//! within the macOS 103-byte Unix socket path limit.  Persistent state
//! (logs, state files) goes to `$XDG_STATE_HOME/agent-procs/sessions/<session>/`.

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
        tracing::warn!(
            path = %path.display(),
            max = MAX_SOCKET_PATH_LEN,
            actual = path.as_os_str().len(),
            "socket path exceeds maximum length; use a shorter session name"
        );
    }
    path
}

pub fn pid_path(session: &str) -> PathBuf {
    socket_base_dir().join(format!("{}.pid", session))
}

/// State directory for persistent data (logs, state.json).
/// Uses $`XDG_STATE_HOME`, defaults to ~/.local/state/.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_socket_base_dir_contains_uid() {
        let uid = nix::unistd::getuid();
        let dir = socket_base_dir();
        let dir_str = dir.to_string_lossy();
        assert!(
            dir_str.contains(&uid.to_string()),
            "socket_base_dir '{}' should contain uid '{}'",
            dir_str,
            uid
        );
    }

    #[test]
    fn test_socket_path_format() {
        let path = socket_path("mysession");
        let path_str = path.to_string_lossy();
        assert!(
            path_str.ends_with(".sock"),
            "socket path '{}' should end with .sock",
            path_str
        );
        assert!(path_str.contains("mysession"));
    }

    #[test]
    fn test_pid_path_format() {
        let path = pid_path("mysession");
        let path_str = path.to_string_lossy();
        assert!(
            path_str.ends_with(".pid"),
            "pid path '{}' should end with .pid",
            path_str
        );
        assert!(path_str.contains("mysession"));
    }

    #[test]
    fn test_log_dir_under_state_dir() {
        let sdir = state_dir("test-session");
        let ldir = log_dir("test-session");
        assert!(
            ldir.starts_with(&sdir),
            "log_dir '{}' should be under state_dir '{}'",
            ldir.display(),
            sdir.display()
        );
    }
}
