mod helpers;
use assert_cmd::Command;
use helpers::TestContext;
use std::time::Duration;

#[test]
fn test_agent_workflow() {
    let ctx = TestContext::new("test-e2e");
    ctx.set_env();

    // 1. Start a "server"
    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args([
            "--session",
            &ctx.session,
            "run",
            "echo 'Starting...' && sleep 0.5 && echo 'Server ready on port 3000' && sleep 60",
            "--name",
            "server",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    // 2. Wait for ready
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

    // 3. Check status (JSON)
    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "status", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let status_out = String::from_utf8_lossy(&output.stdout);
    assert!(
        status_out.contains("running"),
        "expected 'running' in status output: {}",
        status_out
    );

    // 4. Read logs
    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "logs", "server", "--tail", "5"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Server ready"));

    // 5. Stop
    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "stop-all"])
        .output()
        .unwrap();
    assert!(output.status.success());
}
