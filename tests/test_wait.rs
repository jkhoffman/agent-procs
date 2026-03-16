mod helpers;
use assert_cmd::Command;
use helpers::TestContext;
use std::time::Duration;

#[test]
fn test_wait_until_pattern() {
    let ctx = TestContext::new("t-wait-pat");
    ctx.set_env();

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sleep 1 && echo 'Server ready on port 3000' && sleep 60",
            "--name",
            "server",
        ])
        .output()
        .unwrap();

    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args([
            "--session",
            &ctx.session,
            "wait",
            "server",
            "--until",
            "ready on port",
            "--timeout",
            "10",
        ])
        .timeout(Duration::from_secs(15))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "wait failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_wait_exit() {
    let ctx = TestContext::new("t-wait-exit");
    ctx.set_env();

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args([
            "--session",
            &ctx.session,
            "run",
            "echo done && exit 0",
            "--name",
            "quick",
        ])
        .output()
        .unwrap();

    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args([
            "--session",
            &ctx.session,
            "wait",
            "quick",
            "--exit",
            "--timeout",
            "10",
        ])
        .timeout(Duration::from_secs(15))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "wait exit failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_wait_timeout() {
    let ctx = TestContext::new("t-wait-timeout");
    ctx.set_env();

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sleep 60",
            "--name",
            "forever",
        ])
        .output()
        .unwrap();

    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args([
            "--session",
            &ctx.session,
            "wait",
            "forever",
            "--until",
            "this will never appear",
            "--timeout",
            "2",
        ])
        .timeout(Duration::from_secs(10))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}
