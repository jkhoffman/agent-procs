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
fn test_watch_restart_on_file_change() {
    let ctx = TestContext::new("test-watch-restart");

    // Create a tempdir with a file to watch and a Procfile.toml that
    // sets cwd so the watcher monitors the tempdir.
    let watch_dir = tempfile::TempDir::new().unwrap();
    let watched_file = watch_dir.path().join("config.txt");
    std::fs::write(&watched_file, "v1").unwrap();

    let watch_dir_str = watch_dir.path().to_string_lossy();
    let procfile = format!(
        "processes:\n  watcher:\n    cmd: sleep 999\n    cwd: \"{watch_dir_str}\"\n    watch:\n      - \"**\"\n",
    );
    let procfile_path = watch_dir.path().join("agent-procs.yaml");
    std::fs::write(&procfile_path, &procfile).unwrap();

    ctx.cmd()
        .args([
            "--session",
            &ctx.session,
            "up",
            "--config",
            procfile_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    // Let the process start and watcher initialise
    std::thread::sleep(std::time::Duration::from_millis(1000));

    // Verify process is running and reported as watched
    let output = ctx
        .cmd()
        .args(["--session", &ctx.session, "status", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let procs: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let watcher = procs.iter().find(|p| p["name"] == "watcher").unwrap();
    assert_eq!(watcher["state"], "running");
    assert_eq!(watcher["watched"], true);
    let pid_before = watcher["pid"].as_u64().unwrap();

    // Modify the watched file to trigger a restart
    std::fs::write(&watched_file, "v2").unwrap();

    // Wait for debounce (500ms default) + restart time
    std::thread::sleep(std::time::Duration::from_millis(2000));

    // Verify the process is still running (was restarted)
    let output = ctx
        .cmd()
        .args(["--session", &ctx.session, "status", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let procs: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let watcher = procs.iter().find(|p| p["name"] == "watcher").unwrap();
    assert_eq!(watcher["state"], "running");
    // After restart the PID should differ (new process)
    let pid_after = watcher["pid"].as_u64().unwrap();
    assert_ne!(
        pid_before, pid_after,
        "PID should change after watch restart"
    );
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

#[test]
fn test_manual_restart_writes_annotation() {
    let ctx = TestContext::new("test-restart-annotation");
    ctx.cmd()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sleep 999",
            "--name",
            "worker",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(500));

    ctx.cmd()
        .args(["--session", &ctx.session, "restart", "worker"])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(500));

    let output = ctx
        .cmd()
        .args(["--session", &ctx.session, "logs", "worker", "--tail", "10"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("[agent-procs] Restarted (manual)"),
        "expected restart annotation in logs, got: {}",
        stdout
    );
}
