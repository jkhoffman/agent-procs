mod helpers;
use helpers::TestContext;
use std::time::Duration;

#[test]
fn test_wait_until_pattern() {
    let ctx = TestContext::new("t-wait-pat");

    let _ = ctx
        .cmd()
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

    let output = ctx
        .cmd()
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

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_wait_exit() {
    let ctx = TestContext::new("t-wait-exit");

    let _ = ctx
        .cmd()
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

    let output = ctx
        .cmd()
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

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_wait_exit_without_output() {
    let ctx = TestContext::new("t-wait-exit-quiet");

    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sleep 1",
            "--name",
            "quiet",
        ])
        .output()
        .unwrap();

    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "wait",
            "quiet",
            "--exit",
            "--timeout",
            "5",
        ])
        .timeout(Duration::from_secs(10))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "wait exit failed for quiet process: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_wait_timeout() {
    let ctx = TestContext::new("t-wait-timeout");

    let _ = ctx
        .cmd()
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

    let output = ctx
        .cmd()
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

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}
