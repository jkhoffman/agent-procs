use agent_procs::session::*;
use std::io::Write;
use tempfile::TempDir;

#[test]
fn test_is_daemon_alive_returns_false_for_missing_pid_file() {
    let tmp = TempDir::new().unwrap();
    assert!(!is_daemon_alive(&tmp.path().join("daemon.pid")));
}

#[test]
fn test_is_daemon_alive_returns_false_for_dead_pid() {
    let tmp = TempDir::new().unwrap();
    let pid_path = tmp.path().join("daemon.pid");
    let mut f = std::fs::File::create(&pid_path).unwrap();
    writeln!(f, "99999999").unwrap();
    assert!(!is_daemon_alive(&pid_path));
}

#[test]
fn test_is_daemon_alive_returns_true_for_current_process() {
    let tmp = TempDir::new().unwrap();
    let pid_path = tmp.path().join("daemon.pid");
    let mut f = std::fs::File::create(&pid_path).unwrap();
    writeln!(f, "{}", std::process::id()).unwrap();
    assert!(is_daemon_alive(&pid_path));
}

#[test]
fn test_next_process_id_increments() {
    let mut counter = IdCounter::new();
    assert_eq!(counter.next(), "p1");
    assert_eq!(counter.next(), "p2");
    assert_eq!(counter.next(), "p3");
}
