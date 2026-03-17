mod helpers;
use helpers::TestContext;
use std::thread;
use std::time::Duration;

/// Verify that stopping a process kills child processes too (not just the shell).
/// We spawn a shell that launches a subprocess, then stop it and check the subprocess is also gone.
#[test]
fn test_stop_kills_child_processes() {
    let ctx = TestContext::new("t-pgid");

    // Spawn a process that starts a subprocess which writes its PID to a file
    let pid_file = ctx.state_dir.path().join("child.pid");
    let pid_file_str = pid_file.to_str().unwrap();
    let cmd = format!(
        "bash -c 'echo $$ > {} && sleep 60' & sleep 60",
        pid_file_str
    );

    let output = ctx
        .cmd()
        .args(["--session", &ctx.session, "run", &cmd, "--name", "parent"])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Wait for child to write its PID
    thread::sleep(Duration::from_millis(500));

    // Read the child PID
    let child_pid: i32 = std::fs::read_to_string(&pid_file)
        .expect("child pid file not found")
        .trim()
        .parse()
        .expect("invalid pid");

    // Verify child is alive
    assert!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(child_pid), None).is_ok(),
        "child process should be alive before stop"
    );

    // Stop the parent
    let output = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop", "parent"])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Wait a moment for signals to propagate
    thread::sleep(Duration::from_millis(500));

    // Verify child is dead
    assert!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(child_pid), None).is_err(),
        "child process should be dead after stop"
    );

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}
