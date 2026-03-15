use std::env;
use std::path::PathBuf;

pub fn runtime_dir(session: &str) -> PathBuf {
    match env::var("XDG_RUNTIME_DIR") {
        Ok(dir) => PathBuf::from(dir).join("agent-procs/sessions").join(session),
        Err(_) => {
            let uid = nix::unistd::getuid();
            PathBuf::from(format!("/tmp/agent-procs-{}", uid))
                .join("sessions")
                .join(session)
        }
    }
}

pub fn sessions_base_dir() -> PathBuf {
    match env::var("XDG_RUNTIME_DIR") {
        Ok(dir) => PathBuf::from(dir).join("agent-procs/sessions"),
        Err(_) => {
            let uid = nix::unistd::getuid();
            PathBuf::from(format!("/tmp/agent-procs-{}", uid)).join("sessions")
        }
    }
}

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

pub fn socket_path(session: &str) -> PathBuf { runtime_dir(session).join("socket") }
pub fn pid_path(session: &str) -> PathBuf { runtime_dir(session).join("daemon.pid") }
pub fn log_dir(session: &str) -> PathBuf { state_dir(session).join("logs") }
pub fn state_file(session: &str) -> PathBuf { state_dir(session).join("state.json") }
