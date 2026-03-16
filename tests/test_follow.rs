mod helpers;
use assert_cmd::Command;
use helpers::TestContext;
use std::time::Duration;

#[test]
fn test_follow_captures_output() {
    let ctx = TestContext::new("t-follow");
    ctx.set_env();

    // Start a process that waits 1s then outputs lines (gives --follow time to subscribe)
    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run",
            "sleep 1 && for i in 1 2 3 4 5; do echo line-$i; sleep 0.2; done",
            "--name", "counter"])
        .output().unwrap();

    // Follow with a line limit
    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "logs", "counter",
            "--follow", "--lines", "3", "--timeout", "10"])
        .timeout(Duration::from_secs(15))
        .output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("line-1"));
    assert!(stdout.contains("line-2"));
    assert!(stdout.contains("line-3"));
    // Should have stopped after 3 lines
    let line_count = stdout.lines().count();
    assert_eq!(line_count, 3, "expected 3 lines, got {}: {}", line_count, stdout);

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}

#[test]
fn test_follow_timeout() {
    let ctx = TestContext::new("t-fol-tmo");
    ctx.set_env();

    // Start a process that waits 1s then outputs a line (gives --follow time to subscribe)
    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run",
            "sleep 1 && echo started && sleep 60",
            "--name", "quiet"])
        .output().unwrap();

    // Follow with short timeout — should get the initial line then timeout
    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "logs", "quiet",
            "--follow", "--timeout", "4"])
        .timeout(Duration::from_secs(10))
        .output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("started"));

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}

#[test]
fn test_follow_all_processes() {
    let ctx = TestContext::new("t-fol-all");
    ctx.set_env();

    // Both processes wait 1s before output so --follow can subscribe first
    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run",
            "sleep 1 && echo alpha-out && sleep 60", "--name", "alpha"])
        .output().unwrap();
    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run",
            "sleep 1 && echo beta-out && sleep 60", "--name", "beta"])
        .output().unwrap();

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "logs",
            "--all", "--follow", "--lines", "2", "--timeout", "10"])
        .timeout(Duration::from_secs(15))
        .output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[alpha]") || stdout.contains("alpha-out"));
    assert!(stdout.contains("[beta]") || stdout.contains("beta-out"));

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}
