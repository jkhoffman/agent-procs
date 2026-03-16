use std::env;
use tempfile::TempDir;

#[test]
fn test_socket_base_dir_uses_tmp() {
    let result = agent_procs::paths::socket_base_dir();
    let uid = nix::unistd::getuid();
    assert_eq!(result, std::path::PathBuf::from(format!("/tmp/agent-procs-{}", uid)));
}

#[test]
fn test_socket_path() {
    let result = agent_procs::paths::socket_path("mysession");
    let uid = nix::unistd::getuid();
    assert_eq!(result, std::path::PathBuf::from(format!("/tmp/agent-procs-{}/mysession.sock", uid)));
}

#[test]
fn test_pid_path() {
    let result = agent_procs::paths::pid_path("mysession");
    let uid = nix::unistd::getuid();
    assert_eq!(result, std::path::PathBuf::from(format!("/tmp/agent-procs-{}/mysession.pid", uid)));
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
    let expected = std::path::PathBuf::from(format!(
        "{}/.local/state/agent-procs/sessions/test-session", home
    ));
    assert_eq!(result, expected);
}

#[test]
fn test_log_dir() {
    let tmp = TempDir::new().unwrap();
    env::set_var("XDG_STATE_HOME", tmp.path());
    let result = agent_procs::paths::log_dir("mysession");
    assert_eq!(result, tmp.path().join("agent-procs/sessions/mysession/logs"));
    env::remove_var("XDG_STATE_HOME");
}

#[test]
fn test_socket_path_is_short() {
    // Verify socket paths stay well under macOS 103-byte limit
    let path = agent_procs::paths::socket_path("my-long-session-name-that-would-have-failed-before");
    assert!(path.to_string_lossy().len() < 100, "socket path too long: {}", path.display());
}
