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
    let ctx = TestContext::new(&format!("t-sess-cln-{}", std::process::id()));
    let pid_path = agent_procs::paths::pid_path(&ctx.session);
    let socket_path = agent_procs::paths::socket_path(&ctx.session);
    let state_dir = ctx
        .state_dir
        .path()
        .join("agent-procs/sessions")
        .join(&ctx.session);

    std::fs::create_dir_all(pid_path.parent().unwrap()).unwrap();
    std::fs::write(&pid_path, "99999999\n").unwrap();
    std::fs::write(&socket_path, "stale socket sentinel").unwrap();
    std::fs::create_dir_all(state_dir.join("logs")).unwrap();
    std::fs::write(state_dir.join("state.json"), "stale state sentinel").unwrap();
    std::fs::write(state_dir.join("logs/stale.log"), "stale log sentinel").unwrap();

    let output = ctx.cmd().args(["session", "clean"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, format!("cleaned stale session: {}\n", ctx.session));
    assert!(!pid_path.exists());
    assert!(!socket_path.exists());
    assert!(!state_dir.exists());
}
