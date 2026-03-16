use crate::paths;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Spawns the daemon as a detached background process by re-executing the
/// current binary with the `--run-daemon SESSION` internal flag.
/// This avoids the fork-inside-tokio-runtime problem.
pub fn spawn_daemon(session: &str) -> std::io::Result<()> {
    let runtime = paths::runtime_dir(session);
    let state = paths::state_dir(session);
    fs::create_dir_all(&runtime)?;
    fs::create_dir_all(state.join("logs"))?;

    let socket_path = paths::socket_path(session);
    let pid_path = paths::pid_path(session);

    // Remove stale socket if present (ignore ENOENT)
    let _ = fs::remove_file(&socket_path);

    // Re-exec self with a hidden flag to run as daemon
    let exe = std::env::current_exe()?;

    // Use double-fork via shell to fully detach
    let child = Command::new(&exe)
        .args(["--run-daemon", session])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    // Detach: we don't want to wait for it (let OS reap it as orphan → init)
    // Drop the child handle without waiting — it becomes a daemon
    drop(child);

    wait_for_daemon_ready(&pid_path, &socket_path)
}

fn wait_for_daemon_ready(pid_path: &Path, socket_path: &Path) -> std::io::Result<()> {
    // Poll for socket existence, then try to connect to verify it's accepting
    for _ in 0..100 {
        if pid_path.exists() && socket_path.exists() {
            // Try connecting to confirm the server is actually listening
            if std::os::unix::net::UnixStream::connect(socket_path).is_ok() {
                return Ok(());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "daemon did not start within 5s"))
}

/// Entry point called when running as daemon (via --run-daemon flag in main.rs).
/// This runs in a fresh process with no tokio runtime yet.
pub async fn run_daemon(session: &str) {
    let socket_path = paths::socket_path(session);
    let pid_path = paths::pid_path(session);

    // Ensure dirs exist
    let runtime = paths::runtime_dir(session);
    let state = paths::state_dir(session);
    let _ = std::fs::create_dir_all(&runtime);
    let _ = std::fs::create_dir_all(state.join("logs"));

    // Write PID file
    if let Ok(mut f) = std::fs::File::create(&pid_path) {
        use std::io::Write;
        let _ = writeln!(f, "{}", std::process::id());
    }

    super::server::run(session, &socket_path).await;

    let _ = std::fs::remove_file(&socket_path);
    let _ = std::fs::remove_file(&pid_path);
}
