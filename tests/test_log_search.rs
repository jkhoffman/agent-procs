mod helpers;
use helpers::TestContext;
use std::thread;
use std::time::Duration;

#[test]
fn test_grep_returns_only_matching_lines() {
    let ctx = TestContext::new("test-grep-match");

    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sh -c 'echo hello; echo world; echo hello again'",
            "--name",
            "grepper",
        ])
        .output()
        .unwrap();

    thread::sleep(Duration::from_millis(500));

    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "logs",
            "grepper",
            "--tail",
            "100",
            "--grep",
            "hello",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Filter out "--" separator lines that appear between non-adjacent matches
    let lines: Vec<&str> = stdout.lines().filter(|l| *l != "--").collect();
    assert_eq!(
        lines.len(),
        2,
        "expected 2 matching lines, got {}: {:?}",
        lines.len(),
        lines
    );
    assert!(lines[0].contains("hello"));
    assert!(lines[1].contains("hello again"));

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_grep_regex() {
    let ctx = TestContext::new("test-grep-regex");

    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sh -c 'echo ERROR: fail; echo warning: slow; echo info: ok'",
            "--name",
            "regexer",
        ])
        .output()
        .unwrap();

    thread::sleep(Duration::from_millis(500));

    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "logs",
            "regexer",
            "--tail",
            "100",
            "--grep",
            "ERROR|warning",
            "--regex",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "expected 2 regex matches, got {}: {:?}",
        lines.len(),
        lines
    );
    assert!(lines[0].contains("ERROR: fail"));
    assert!(lines[1].contains("warning: slow"));

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_grep_tail_returns_last_n_matches() {
    let ctx = TestContext::new("test-grep-tail");

    // Echoes interleaved error and ok lines
    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sh -c 'echo error 1; echo ok; echo error 2; echo ok; echo error 3; echo ok; echo error 4; echo ok; echo error 5'",
            "--name",
            "tailer",
        ])
        .output()
        .unwrap();

    thread::sleep(Duration::from_millis(500));

    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "logs",
            "tailer",
            "--tail",
            "2",
            "--grep",
            "error",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Filter out "--" separator lines that appear between non-adjacent matches
    let lines: Vec<&str> = stdout.lines().filter(|l| *l != "--").collect();
    assert_eq!(
        lines.len(),
        2,
        "expected 2 tail matches, got {}: {:?}",
        lines.len(),
        lines
    );
    assert!(
        lines[0].contains("error 4"),
        "expected 'error 4', got: {}",
        lines[0]
    );
    assert!(
        lines[1].contains("error 5"),
        "expected 'error 5', got: {}",
        lines[1]
    );

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_grep_no_matches_returns_empty() {
    let ctx = TestContext::new("test-grep-empty");

    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sh -c 'echo hello'",
            "--name",
            "nomatch",
        ])
        .output()
        .unwrap();

    thread::sleep(Duration::from_millis(500));

    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "logs",
            "nomatch",
            "--tail",
            "100",
            "--grep",
            "nonexistent",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim().is_empty(),
        "expected empty output, got: {:?}",
        stdout
    );

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_grep_invalid_regex_returns_error() {
    let ctx = TestContext::new("test-grep-badregex");

    // Need a running process for the logs command to have a valid target
    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sh -c 'echo hello'",
            "--name",
            "dummy",
        ])
        .output()
        .unwrap();

    thread::sleep(Duration::from_millis(500));

    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "logs",
            "dummy",
            "--grep",
            "invalid[regex",
            "--regex",
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "expected failure for invalid regex, but got success"
    );

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_context_without_grep_is_error() {
    let ctx = TestContext::new("test-ctx-no-grep");

    // Need a running process
    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sh -c 'echo hello'",
            "--name",
            "dummy",
        ])
        .output()
        .unwrap();

    thread::sleep(Duration::from_millis(500));

    let output = ctx
        .cmd()
        .args(["--session", &ctx.session, "logs", "dummy", "--context", "3"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "expected failure when --context used without --grep"
    );

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_grep_json_output() {
    let ctx = TestContext::new("test-grep-json");

    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sh -c 'echo hello; echo world'",
            "--name",
            "jsonproc",
        ])
        .output()
        .unwrap();

    thread::sleep(Duration::from_millis(500));

    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "logs",
            "jsonproc",
            "--tail",
            "100",
            "--grep",
            "hello",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "expected 1 JSON line, got {}: {:?}",
        lines.len(),
        lines
    );

    let val: serde_json::Value = serde_json::from_str(lines[0]).expect("valid JSON");
    assert_eq!(val["process"], "jsonproc");
    assert_eq!(val["stream"], "stdout");
    assert!(
        val["text"].as_str().unwrap().contains("hello"),
        "text field should contain 'hello', got: {}",
        val["text"]
    );
    assert!(
        val["line_number"].is_number(),
        "line_number should be a number"
    );

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_since_start_returns_all() {
    let ctx = TestContext::new("test-since-start");

    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sh -c 'echo line1; echo line2; echo line3'",
            "--name",
            "alllines",
        ])
        .output()
        .unwrap();

    thread::sleep(Duration::from_millis(500));

    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "logs",
            "alllines",
            "--tail",
            "100",
            "--since",
            "start",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("line1"), "should contain line1");
    assert!(stdout.contains("line2"), "should contain line2");
    assert!(stdout.contains("line3"), "should contain line3");
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(
        lines.len() >= 3,
        "expected at least 3 lines, got {}: {:?}",
        lines.len(),
        lines
    );

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_since_restart_after_manual_restart() {
    let ctx = TestContext::new("test-since-restart");

    // Start a process that echoes and then exits, so log indexes get flushed
    ctx.cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sh -c 'echo pre-restart-line; sleep 999'",
            "--name",
            "worker",
        ])
        .assert()
        .success();

    thread::sleep(Duration::from_millis(500));

    // Restart the process — this writes a "[agent-procs] Restarted (manual)"
    // annotation to the NEW log file (old log is truncated by File::create)
    ctx.cmd()
        .args(["--session", &ctx.session, "restart", "worker"])
        .assert()
        .success();

    // Wait for restart to complete
    thread::sleep(Duration::from_millis(1000));

    // First verify the restart marker exists in the full log via --tail
    let output = ctx
        .cmd()
        .args(["--session", &ctx.session, "logs", "worker", "--tail", "100"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let full_stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        full_stdout.contains("[agent-procs] Restarted (manual)"),
        "full log should contain restart marker, got: {}",
        full_stdout
    );

    // The pre-restart output should NOT be in the log (restart truncates the file)
    assert!(
        !full_stdout.contains("pre-restart-line"),
        "pre-restart output should not survive restart truncation, got: {}",
        full_stdout
    );

    // Now check --since restart: it starts AFTER the marker line,
    // so the marker itself should be excluded from output
    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "logs",
            "worker",
            "--tail",
            "100",
            "--since",
            "restart",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // --since restart starts at marker_line + 1, excluding the marker
    assert!(
        !stdout.contains("[agent-procs] Restarted (manual)"),
        "should not contain restart marker (--since restart starts after it), got: {}",
        stdout
    );

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_since_restart_no_prior_restart() {
    let ctx = TestContext::new("test-since-norestart");

    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sh -c 'echo output-line'",
            "--name",
            "oneshot",
        ])
        .output()
        .unwrap();

    thread::sleep(Duration::from_millis(500));

    // --since restart with no prior restart should return all output
    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "logs",
            "oneshot",
            "--tail",
            "100",
            "--since",
            "restart",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("output-line"),
        "should contain all output when no restart marker exists, got: {}",
        stdout
    );

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_grep_context_with_separators() {
    let ctx = TestContext::new("test-grep-ctx-sep");

    // Echo 8 lines: lines 0-7, with ERROR at lines 1 and 6 (0-indexed)
    // so there are several lines between the two ERRORs
    let _ = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sh -c 'echo line0; echo ERROR first; echo line2; echo line3; echo line4; echo line5; echo ERROR second; echo line7'",
            "--name",
            "ctxproc",
        ])
        .output()
        .unwrap();

    thread::sleep(Duration::from_millis(500));

    let output = ctx
        .cmd()
        .args([
            "--session",
            &ctx.session,
            "logs",
            "ctxproc",
            "--tail",
            "100",
            "--grep",
            "ERROR",
            "--context",
            "1",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain the ERROR lines
    assert!(
        stdout.contains("ERROR first"),
        "should contain 'ERROR first', got: {}",
        stdout
    );
    assert!(
        stdout.contains("ERROR second"),
        "should contain 'ERROR second', got: {}",
        stdout
    );

    // Should contain context lines around matches
    assert!(
        stdout.contains("line0"),
        "should contain context line 'line0', got: {}",
        stdout
    );
    assert!(
        stdout.contains("line2"),
        "should contain context line 'line2', got: {}",
        stdout
    );
    assert!(
        stdout.contains("line5"),
        "should contain context line 'line5', got: {}",
        stdout
    );
    assert!(
        stdout.contains("line7"),
        "should contain context line 'line7', got: {}",
        stdout
    );

    // Should have a separator between the two non-adjacent groups
    assert!(
        stdout.contains("--"),
        "should contain '--' separator between groups, got: {}",
        stdout
    );

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}
