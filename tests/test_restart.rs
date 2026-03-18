mod helpers;
use helpers::TestContext;

#[test]
fn test_autorestart_on_failure() {
    let ctx = TestContext::new("test-autorestart");
    ctx.cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sh -c 'exit 1'",
            "--name",
            "crasher",
            "--autorestart",
            "on-failure",
            "--max-restarts",
            "2",
            "--restart-delay",
            "200",
        ])
        .assert()
        .success();

    // Wait for restart to happen (200ms delay + margin)
    std::thread::sleep(std::time::Duration::from_millis(1500));

    let output = ctx
        .cmd()
        .args(["--session", &ctx.session, "status", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let procs: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let crasher = procs.iter().find(|p| p["name"] == "crasher").unwrap();
    assert!(crasher["restart_count"].as_u64().unwrap() > 0);
}

#[test]
fn test_autorestart_skips_clean_exit() {
    let ctx = TestContext::new("test-autorestart-clean");
    ctx.cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sh -c 'exit 0'",
            "--name",
            "clean",
            "--autorestart",
            "on-failure",
        ])
        .assert()
        .success();

    std::thread::sleep(std::time::Duration::from_millis(1500));

    let output = ctx
        .cmd()
        .args(["--session", &ctx.session, "status", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let procs: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let clean = procs.iter().find(|p| p["name"] == "clean").unwrap();
    assert_eq!(clean["restart_count"].as_u64(), None); // None because 0 is omitted
    assert_eq!(clean["state"], "exited");
}

#[test]
fn test_max_restarts_exhausted() {
    let ctx = TestContext::new("test-max-restarts");
    ctx.cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sh -c 'exit 1'",
            "--name",
            "crasher",
            "--autorestart",
            "always",
            "--max-restarts",
            "1",
            "--restart-delay",
            "200",
        ])
        .assert()
        .success();

    // Wait for restart attempts to exhaust
    std::thread::sleep(std::time::Duration::from_millis(2000));

    let output = ctx
        .cmd()
        .args(["--session", &ctx.session, "status", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let procs: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let crasher = procs.iter().find(|p| p["name"] == "crasher").unwrap();
    assert_eq!(crasher["state"], "failed");
}

#[test]
fn test_stop_disables_autorestart() {
    let ctx = TestContext::new("test-stop-restart");
    ctx.cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sleep 999",
            "--name",
            "sleeper",
            "--autorestart",
            "always",
        ])
        .assert()
        .success();

    std::thread::sleep(std::time::Duration::from_millis(500));

    ctx.cmd()
        .args(["--session", &ctx.session, "stop", "sleeper"])
        .assert()
        .success();

    std::thread::sleep(std::time::Duration::from_millis(1500));

    let output = ctx
        .cmd()
        .args(["--session", &ctx.session, "status", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let procs: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let sleeper = procs.iter().find(|p| p["name"] == "sleeper").unwrap();
    assert_eq!(sleeper["state"], "exited");
}
