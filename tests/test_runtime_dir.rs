//! The per-user runtime dir (socket and PID files) must be a real directory
//! owned by the current user.  These tests run the real CLI and daemon, with
//! the runtime dir redirected via a debug-build-only environment variable.

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn cli(runtime: &Path, state: &Path) -> Command {
    let mut cmd = Command::cargo_bin("agent-procs").unwrap();
    cmd.env("AGENT_PROCS_TEST_RUNTIME_DIR", runtime)
        .env("XDG_STATE_HOME", state);
    cmd
}

fn kill_daemon(runtime: &Path, session: &str) {
    if let Ok(pid) = fs::read_to_string(runtime.join(format!("{session}.pid")))
        && let Ok(pid) = pid.trim().parse::<i32>()
    {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGTERM,
        );
    }
}

/// Release builds ignore the override, so these tests only apply to debug.
fn override_supported() -> bool {
    cfg!(debug_assertions)
}

#[test]
fn symlinked_runtime_dir_is_refused_and_target_untouched() {
    if !override_supported() {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let target = tmp.path().join("attacker");
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    let link = tmp.path().join("rt");
    symlink(&target, &link).unwrap();

    // Read-only command must not trust the path.
    cli(&link, state.path())
        .args(["--session", "rd-symlink", "status"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "refusing to use runtime directory",
        ));

    // Auto-spawning command must not start a daemon inside the target.
    cli(&link, state.path())
        .args(["--session", "rd-symlink", "run", "sleep 30"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "refusing to use runtime directory",
        ));

    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o755);
}

#[test]
fn run_daemon_refuses_symlinked_runtime_dir_before_writing_anything() {
    if !override_supported() {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("attacker");
    fs::create_dir(&target).unwrap();
    let link = tmp.path().join("rt");
    symlink(&target, &link).unwrap();
    // HOME unset: the state dir would resolve inside the runtime dir.
    let mut cmd = Command::cargo_bin("agent-procs").unwrap();
    cmd.env("AGENT_PROCS_TEST_RUNTIME_DIR", &link)
        .env_remove("HOME")
        .env_remove("XDG_STATE_HOME")
        .args(["run-daemon", "rd-direct"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "refusing to use runtime directory",
        ));
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
}

#[test]
fn dangling_symlink_runtime_dir_is_refused() {
    if !override_supported() {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let link = tmp.path().join("rt");
    symlink(tmp.path().join("missing"), &link).unwrap();
    cli(&link, state.path())
        .args(["--session", "rd-dangling", "run", "sleep 30"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "refusing to use runtime directory",
        ));
    assert!(!tmp.path().join("missing").exists());
}

#[test]
fn loose_runtime_dir_owned_by_us_is_tightened_and_works() {
    if !override_supported() {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let rt = tmp.path().join("rt");
    // Simulates a directory created by an older version with a looser mode.
    fs::create_dir(&rt).unwrap();
    fs::set_permissions(&rt, fs::Permissions::from_mode(0o755)).unwrap();

    cli(&rt, state.path())
        .args(["--session", "rd-loose", "run", "sleep 30", "--name", "s"])
        .assert()
        .success();
    assert_eq!(fs::metadata(&rt).unwrap().mode() & 0o777, 0o700);
    cli(&rt, state.path())
        .args(["--session", "rd-loose", "status"])
        .assert()
        .success();

    cli(&rt, state.path())
        .args(["--session", "rd-loose", "stop-all"])
        .assert()
        .success();
    kill_daemon(&rt, "rd-loose");
}

#[test]
fn state_without_home_lands_in_private_runtime_dir() {
    if !override_supported() {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let rt = tmp.path().join("rt");
    let mut cmd = Command::cargo_bin("agent-procs").unwrap();
    cmd.env("AGENT_PROCS_TEST_RUNTIME_DIR", &rt)
        .env_remove("HOME")
        .env_remove("XDG_STATE_HOME")
        .args(["--session", "rd-nohome", "run", "sleep 30", "--name", "s"])
        .assert()
        .success();
    assert!(rt.join("state/agent-procs/sessions/rd-nohome").is_dir());
    assert_eq!(fs::metadata(&rt).unwrap().mode() & 0o777, 0o700);
    let mut stop = Command::cargo_bin("agent-procs").unwrap();
    stop.env("AGENT_PROCS_TEST_RUNTIME_DIR", &rt)
        .env_remove("HOME")
        .env_remove("XDG_STATE_HOME")
        .args(["--session", "rd-nohome", "stop-all"])
        .assert()
        .success();
    kill_daemon(&rt, "rd-nohome");
}

#[test]
fn missing_runtime_dir_is_created_private() {
    if !override_supported() {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let rt = tmp.path().join("rt");

    cli(&rt, state.path())
        .args(["--session", "rd-new", "run", "sleep 30", "--name", "s"])
        .assert()
        .success();
    let meta = fs::metadata(&rt).unwrap();
    assert!(meta.is_dir());
    assert_eq!(meta.mode() & 0o777, 0o700);

    cli(&rt, state.path())
        .args(["--session", "rd-new", "stop-all"])
        .assert()
        .success();
    kill_daemon(&rt, "rd-new");
}

/// Needs root to give a directory to another uid, so it only runs when the
/// test suite is run as root (for example in a container).
#[test]
fn runtime_dir_owned_by_another_uid_is_refused() {
    if !override_supported() || !nix::unistd::geteuid().is_root() {
        eprintln!("skipped: requires root");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let rt = tmp.path().join("rt");
    fs::create_dir(&rt).unwrap();
    std::os::unix::fs::chown(&rt, Some(65534), Some(65534)).unwrap();

    cli(&rt, state.path())
        .args(["--session", "rd-other", "status"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("owned by uid 65534"));
    cli(&rt, state.path())
        .args(["--session", "rd-other", "run", "sleep 30"])
        .assert()
        .failure()
        .code(1);
    assert_eq!(fs::read_dir(&rt).unwrap().count(), 0);
}
