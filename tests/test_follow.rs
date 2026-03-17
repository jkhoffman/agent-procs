mod helpers;
use helpers::TestContext;
use std::time::Duration;

#[test]
fn test_follow_captures_output() {
    let ctx = TestContext::new("t-follow");

    // Start a process that waits 1s then outputs lines (gives --follow time to subscribe)
    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sleep 1 && for i in 1 2 3 4 5; do echo line-$i; sleep 0.2; done",
            "--name",
            "counter",
        ])
        .output()
        .unwrap();

    // Follow with a line limit
    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "logs",
            "counter",
            "--follow",
            "--lines",
            "3",
            "--timeout",
            "10",
        ])
        .timeout(Duration::from_secs(15))
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("line-1"));
    assert!(stdout.contains("line-2"));
    assert!(stdout.contains("line-3"));
    // Should have stopped after 3 lines
    let line_count = stdout.lines().count();
    assert_eq!(
        line_count, 3,
        "expected 3 lines, got {}: {}",
        line_count, stdout
    );

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_follow_timeout() {
    let ctx = TestContext::new("t-fol-tmo");

    // Start a process that waits 1s then outputs a line (gives --follow time to subscribe)
    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sleep 1 && echo started && sleep 60",
            "--name",
            "quiet",
        ])
        .output()
        .unwrap();

    // Follow with short timeout — should get the initial line then timeout
    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "logs",
            "quiet",
            "--follow",
            "--timeout",
            "4",
        ])
        .timeout(Duration::from_secs(10))
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("started"));

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_follow_all_processes() {
    let ctx = TestContext::new("t-fol-all");

    // Both processes wait 2s before output so --follow can subscribe first.
    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sleep 2 && echo alpha-out && sleep 60",
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
            "sleep 2 && echo beta-out && sleep 60",
            "--name",
            "beta",
        ])
        .output()
        .unwrap();

    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "logs",
            "--all",
            "--follow",
            "--lines",
            "2",
            "--timeout",
            "10",
        ])
        .timeout(Duration::from_secs(15))
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[alpha]") || stdout.contains("alpha-out"));
    assert!(stdout.contains("[beta]") || stdout.contains("beta-out"));

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_follow_stderr_only() {
    let ctx = TestContext::new("t-fol-stderr");

    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sleep 1 && echo stdout-line && echo stderr-line >&2 && sleep 60",
            "--name",
            "mixed",
        ])
        .output()
        .unwrap();

    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "logs",
            "mixed",
            "--follow",
            "--stderr",
            "--lines",
            "1",
            "--timeout",
            "10",
        ])
        .timeout(Duration::from_secs(15))
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("stderr-line"));
    assert!(!stdout.contains("stdout-line"));

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_follow_replays_tail_before_streaming() {
    let ctx = TestContext::new("t-fol-tail");

    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "echo prelude && sleep 1 && echo after && sleep 60",
            "--name",
            "tailer",
        ])
        .output()
        .unwrap();

    let mut prelude_ready = false;
    for _ in 0..20 {
        let output = ctx
            .cmd()
            .args(["--session", &ctx.session, "logs", "tailer", "--tail", "1"])
            .output()
            .unwrap();
        if output.status.success() && String::from_utf8_lossy(&output.stdout).contains("prelude") {
            prelude_ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(prelude_ready, "prelude line never reached disk");

    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "logs",
            "tailer",
            "--follow",
            "--tail",
            "1",
            "--lines",
            "1",
            "--timeout",
            "10",
        ])
        .timeout(Duration::from_secs(15))
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("prelude"),
        "tail replay missing: {}",
        stdout
    );
    assert!(stdout.contains("after"), "live follow missing: {}", stdout);

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}
