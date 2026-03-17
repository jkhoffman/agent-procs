mod helpers;
use helpers::TestContext;
use std::thread;
use std::time::Duration;

#[test]
fn test_logs_tail() {
    let ctx = TestContext::new("test-logs-tail");

    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "echo hello world",
            "--name",
            "echoer",
        ])
        .output()
        .unwrap();

    thread::sleep(Duration::from_millis(500));

    let output = ctx
        .cmd()
        .args(["--session", &ctx.session, "logs", "echoer", "--tail", "10"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("hello world"));

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_logs_all_interleaved() {
    let ctx = TestContext::new("test-logs-all");

    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "echo from-alpha",
            "--name",
            "alpha",
        ])
        .output()
        .unwrap();
    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "echo from-beta",
            "--name",
            "beta",
        ])
        .output()
        .unwrap();

    thread::sleep(Duration::from_millis(500));

    let output = ctx
        .cmd()
        .args(["--session", &ctx.session, "logs", "--all", "--tail", "10"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[alpha]"));
    assert!(stdout.contains("[beta]"));

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}
