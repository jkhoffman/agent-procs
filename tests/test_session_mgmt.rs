mod helpers;
use helpers::TestContext;

#[test]
fn test_session_list_shows_active() {
    let ctx = TestContext::new("test-sess-list");

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "run", "sleep 60", "--name", "bg"])
        .output()
        .unwrap();

    let output = ctx.cmd().args(["session", "list"]).output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains(&ctx.session));

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_session_clean_removes_stale() {
    let ctx = TestContext::new("t-sess-cln");

    // Start and stop to create a session
    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "run", "sleep 60", "--name", "bg"])
        .output()
        .unwrap();
    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();

    // Write a fake PID to make it look stale
    let pid_path = agent_procs::paths::pid_path(&ctx.session);
    if pid_path.exists() {
        std::fs::write(&pid_path, "99999999\n").unwrap();
    }

    let output = ctx.cmd().args(["session", "clean"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cleaned"));
}
