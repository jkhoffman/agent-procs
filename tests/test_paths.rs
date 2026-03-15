use std::env;
use tempfile::TempDir;

#[test]
fn test_runtime_dir_uses_xdg_when_set() {
    let tmp = TempDir::new().unwrap();
    env::set_var("XDG_RUNTIME_DIR", tmp.path());
    let result = agent_procs::paths::runtime_dir("test-session");
    assert_eq!(result, tmp.path().join("agent-procs/sessions/test-session"));
    env::remove_var("XDG_RUNTIME_DIR");
}

#[test]
fn test_runtime_dir_falls_back_to_tmp() {
    env::remove_var("XDG_RUNTIME_DIR");
    let result = agent_procs::paths::runtime_dir("test-session");
    let uid = nix::unistd::getuid();
    let expected = std::path::PathBuf::from(format!("/tmp/agent-procs-{}/sessions/test-session", uid));
    assert_eq!(result, expected);
}

#[test]
fn test_sessions_base_dir() {
    let tmp = TempDir::new().unwrap();
    env::set_var("XDG_RUNTIME_DIR", tmp.path());
    let result = agent_procs::paths::sessions_base_dir();
    assert_eq!(result, tmp.path().join("agent-procs/sessions"));
    env::remove_var("XDG_RUNTIME_DIR");
}

#[test]
fn test_state_dir_uses_xdg_when_set() {
    let tmp = TempDir::new().unwrap();
    env::set_var("XDG_STATE_HOME", tmp.path());
    let result = agent_procs::paths::state_dir("test-session");
    assert_eq!(result, tmp.path().join("agent-procs/sessions/test-session"));
    env::remove_var("XDG_STATE_HOME");
}

#[test]
fn test_state_dir_defaults_to_home() {
    env::remove_var("XDG_STATE_HOME");
    let result = agent_procs::paths::state_dir("test-session");
    let home = env::var("HOME").unwrap();
    let expected = std::path::PathBuf::from(format!("{}/.local/state/agent-procs/sessions/test-session", home));
    assert_eq!(result, expected);
}

#[test]
fn test_socket_path() {
    let tmp = TempDir::new().unwrap();
    env::set_var("XDG_RUNTIME_DIR", tmp.path());
    let result = agent_procs::paths::socket_path("mysession");
    assert_eq!(result, tmp.path().join("agent-procs/sessions/mysession/socket"));
    env::remove_var("XDG_RUNTIME_DIR");
}

#[test]
fn test_pid_path() {
    let tmp = TempDir::new().unwrap();
    env::set_var("XDG_RUNTIME_DIR", tmp.path());
    let result = agent_procs::paths::pid_path("mysession");
    assert_eq!(result, tmp.path().join("agent-procs/sessions/mysession/daemon.pid"));
    env::remove_var("XDG_RUNTIME_DIR");
}

#[test]
fn test_log_dir() {
    let tmp = TempDir::new().unwrap();
    env::set_var("XDG_STATE_HOME", tmp.path());
    let result = agent_procs::paths::log_dir("mysession");
    assert_eq!(result, tmp.path().join("agent-procs/sessions/mysession/logs"));
    env::remove_var("XDG_STATE_HOME");
}
