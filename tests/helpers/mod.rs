use assert_cmd::Command;
use tempfile::TempDir;

pub struct TestContext {
    pub state_dir: TempDir,
    pub session: String,
}

impl TestContext {
    pub fn new(session: &str) -> Self {
        // Kill any stale daemon from a previous test run before starting fresh
        kill_daemon_for_session(session);
        Self {
            state_dir: TempDir::new().unwrap(),
            session: session.into(),
        }
    }

    pub fn cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("agent-procs").unwrap();
        cmd.env("XDG_STATE_HOME", self.state_dir.path());
        cmd
    }
}

impl Drop for TestContext {
    fn drop(&mut self) {
        // Kill the daemon when the test context is dropped (end of test)
        kill_daemon_for_session(&self.session);
    }
}

fn kill_daemon_for_session(session: &str) {
    let uid = nix::unistd::getuid();
    let pid_file = format!("/tmp/agent-procs-{}/{}.pid", uid, session);
    if let Ok(contents) = std::fs::read_to_string(&pid_file) {
        if let Ok(pid) = contents.trim().parse::<i32>() {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGTERM,
            );
        }
        let _ = std::fs::remove_file(&pid_file);
        let _ = std::fs::remove_file(format!("/tmp/agent-procs-{}/{}.sock", uid, session));
    }
}
