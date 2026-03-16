mod helpers;
use assert_cmd::Command;
use helpers::TestContext;

// All integration tests MUST run with --test-threads=1 (env var mutation).

#[test]
fn test_run_and_status() {
    let ctx = TestContext::new("test-run-status");
    ctx.set_env();

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run", "sleep 60", "--name", "sleeper"])
        .output().unwrap();
    assert!(output.status.success(), "run failed: {}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("sleeper"));

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "status"])
        .output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("sleeper"));
    assert!(stdout.contains("running"));

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop", "sleeper"])
        .output().unwrap();
    assert!(output.status.success());

    // Cleanup daemon
    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_stop_nonexistent_returns_error() {
    let ctx = TestContext::new("test-stop-missing");
    ctx.set_env();

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run", "sleep 60", "--name", "bg"])
        .output().unwrap();

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop", "nonexistent"])
        .output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not found") || !output.status.success());

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}

#[test]
fn test_restart_process() {
    let ctx = TestContext::new("t-restart");
    ctx.set_env();

    // Start a process
    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run", "sleep 60", "--name", "srv"])
        .output().unwrap();
    assert!(output.status.success());

    // Get original PID from status
    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "status", "--json"])
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let procs: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let original_pid = procs[0]["pid"].as_u64().unwrap();

    // Restart it
    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "restart", "srv"])
        .output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("restarted"));

    // Verify it's running with a new PID
    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "status", "--json"])
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let procs: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let new_pid = procs[0]["pid"].as_u64().unwrap();
    assert_ne!(original_pid, new_pid, "PID should change after restart");

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}
