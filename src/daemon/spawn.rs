use crate::paths;
use std::fs;
use std::io::Write;
use std::path::Path;

pub fn spawn_daemon(session: &str) -> std::io::Result<()> {
    let runtime = paths::runtime_dir(session);
    let state = paths::state_dir(session);
    fs::create_dir_all(&runtime)?;
    fs::create_dir_all(state.join("logs"))?;

    let socket_path = paths::socket_path(session);
    let pid_path = paths::pid_path(session);

    if socket_path.exists() {
        fs::remove_file(&socket_path)?;
    }

    // Fork 1: parent returns, child continues
    match unsafe { nix::unistd::fork() } {
        Ok(nix::unistd::ForkResult::Parent { .. }) => {
            wait_for_daemon_ready(&pid_path, &socket_path)?;
            return Ok(());
        }
        Ok(nix::unistd::ForkResult::Child) => {}
        Err(e) => return Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
    }

    nix::unistd::setsid().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    // Fork 2: first child exits, grandchild is the daemon
    match unsafe { nix::unistd::fork() } {
        Ok(nix::unistd::ForkResult::Parent { .. }) => std::process::exit(0),
        Ok(nix::unistd::ForkResult::Child) => {}
        Err(e) => return Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
    }

    let mut f = fs::File::create(&pid_path)?;
    writeln!(f, "{}", std::process::id())?;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(super::server::run(session, &socket_path));

    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&pid_path);
    std::process::exit(0);
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
