//! Socket, PID file, and log directory path resolution.
//!
//! Sockets and PID files live under `/tmp/agent-procs-<uid>/` to stay
//! within the macOS 103-byte Unix socket path limit.  Persistent state
//! (logs, state files) goes to `$XDG_STATE_HOME/agent-procs/sessions/<session>/`.

use std::env;
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// macOS limits Unix socket paths to 103 bytes; Linux allows 107.
const MAX_SOCKET_PATH_LEN: usize = 103;

/// Base directory for sockets and PID files: /tmp/agent-procs-<uid>/
/// Short fixed path to avoid macOS 103-byte socket path limit.
pub fn socket_base_dir() -> PathBuf {
    // Debug builds only: lets integration tests point the CLI at a scratch
    // directory instead of the real per-user runtime dir.  Release builds
    // never read this variable.
    #[cfg(debug_assertions)]
    if let Some(dir) = env::var_os("AGENT_PROCS_TEST_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    // Same uid source as the ownership checks in `check_private_dir`.
    let uid = nix::unistd::geteuid();
    PathBuf::from(format!("/tmp/agent-procs-{}", uid))
}

/// Create the socket base directory (mode 0700) if needed, then verify it.
///
/// See [`verify_socket_dir`] for the checks applied.
pub fn ensure_socket_dir() -> io::Result<()> {
    let dir = socket_base_dir();
    match fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
    {
        Ok(()) => {}
        // Something already exists at the path (possibly a dangling symlink).
        // `create` does not vet it; `check_private_dir` below does.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    check_private_dir(&dir, nix::unistd::geteuid().as_raw())
}

/// Verify that the socket base directory is safe to use without creating it.
///
/// Returns `NotFound` if it does not exist.  Otherwise the path must be a real
/// directory (not a symlink) owned by the current effective uid.  A directory
/// we own with group/world access (for example one created by an older
/// version) is tightened to 0700; one we do not own is refused.
pub fn verify_socket_dir() -> io::Result<()> {
    check_private_dir(&socket_base_dir(), nix::unistd::geteuid().as_raw())
}

/// Checks run on an open descriptor, so the verified object is the one we
/// tighten: no window between lstat and chmod.
fn check_private_dir(dir: &Path, euid: u32) -> io::Result<()> {
    let refuse = |why: String| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "refusing to use runtime directory {}: {}",
                dir.display(),
                why
            ),
        )
    };
    let handle = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_DIRECTORY | nix::libc::O_CLOEXEC)
        .open(dir)
        .map_err(|e| match e.raw_os_error() {
            Some(code) if code == nix::libc::ELOOP || code == nix::libc::ENOTDIR => {
                refuse("not a real directory (symlink or file)".into())
            }
            Some(code) if code == nix::libc::EACCES => {
                refuse("cannot open it (permission denied)".into())
            }
            _ => e,
        })?;
    let meta = handle.metadata()?;
    if meta.uid() != euid {
        return Err(refuse(format!(
            "owned by uid {}, expected {}",
            meta.uid(),
            euid
        )));
    }
    if meta.mode() & 0o077 != 0 {
        handle.set_permissions(fs::Permissions::from_mode(0o700))?;
        if handle.metadata()?.mode() & 0o077 != 0 {
            return Err(refuse("could not restrict permissions to 0700".into()));
        }
    }
    Ok(())
}

pub fn socket_path(session: &str) -> PathBuf {
    let path = socket_base_dir().join(format!("{}.sock", session));
    if path.as_os_str().len() > MAX_SOCKET_PATH_LEN {
        tracing::warn!(
            path = %path.display(),
            max = MAX_SOCKET_PATH_LEN,
            actual = path.as_os_str().len(),
            "socket path exceeds maximum length; use a shorter session name"
        );
    }
    path
}

pub fn pid_path(session: &str) -> PathBuf {
    socket_base_dir().join(format!("{}.pid", session))
}

/// State directory for persistent data (logs, state.json).
/// Uses $`XDG_STATE_HOME`, defaults to ~/.local/state/ (or `<runtime dir>/state`
/// when HOME is unset).
pub fn state_dir(session: &str) -> PathBuf {
    let base = match env::var("XDG_STATE_HOME") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => match env::var("HOME") {
            Ok(home) => PathBuf::from(home).join(".local/state"),
            // No HOME: use the verified private runtime dir, never a
            // predictable path in shared /tmp.
            Err(_) => socket_base_dir().join("state"),
        },
    };
    base.join("agent-procs/sessions").join(session)
}

pub fn log_dir(session: &str) -> PathBuf {
    state_dir(session).join("logs")
}
pub fn state_file(session: &str) -> PathBuf {
    state_dir(session).join("state.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn euid() -> u32 {
        nix::unistd::geteuid().as_raw()
    }

    #[test]
    fn check_private_dir_accepts_own_0700_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("rt");
        fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        check_private_dir(&dir, euid()).unwrap();
    }

    #[test]
    fn check_private_dir_tightens_own_loose_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("rt");
        fs::DirBuilder::new().mode(0o755).create(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        check_private_dir(&dir, euid()).unwrap();
        assert_eq!(fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
    }

    #[test]
    fn check_private_dir_refuses_other_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("rt");
        fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let err = check_private_dir(&dir, euid() + 1).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(err.to_string().contains("owned by uid"), "{err}");
        // A directory we do not own must not be chmod'ed.
        assert_eq!(fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
    }

    #[test]
    fn check_private_dir_refuses_symlink_and_leaves_target_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        fs::DirBuilder::new().mode(0o755).create(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        let link = tmp.path().join("link");
        symlink(&target, &link).unwrap();
        let err = check_private_dir(&link, euid()).unwrap_err();
        assert!(err.to_string().contains("not a real directory"), "{err}");
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o755);
    }

    #[test]
    fn check_private_dir_refuses_unreadable_dir_with_clear_message() {
        if nix::unistd::geteuid().is_root() {
            return; // root ignores mode bits
        }
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("rt");
        fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).unwrap();
        let err = check_private_dir(&dir, euid()).unwrap_err();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            err.to_string()
                .contains("refusing to use runtime directory"),
            "{err}"
        );
    }

    #[test]
    fn check_private_dir_refuses_regular_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("f");
        fs::write(&file, b"x").unwrap();
        let err = check_private_dir(&file, euid()).unwrap_err();
        assert!(err.to_string().contains("not a real directory"), "{err}");
    }

    #[test]
    fn check_private_dir_missing_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let err = check_private_dir(&tmp.path().join("nope"), euid()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn test_socket_base_dir_contains_uid() {
        let uid = nix::unistd::getuid();
        let dir = socket_base_dir();
        let dir_str = dir.to_string_lossy();
        assert!(
            dir_str.contains(&uid.to_string()),
            "socket_base_dir '{}' should contain uid '{}'",
            dir_str,
            uid
        );
    }

    #[test]
    fn test_socket_path_format() {
        let path = socket_path("mysession");
        let path_str = path.to_string_lossy();
        assert!(
            path_str.ends_with(".sock"),
            "socket path '{}' should end with .sock",
            path_str
        );
        assert!(path_str.contains("mysession"));
    }

    #[test]
    fn test_pid_path_format() {
        let path = pid_path("mysession");
        let path_str = path.to_string_lossy();
        assert!(
            path_str.ends_with(".pid"),
            "pid path '{}' should end with .pid",
            path_str
        );
        assert!(path_str.contains("mysession"));
    }

    #[test]
    fn test_log_dir_under_state_dir() {
        let sdir = state_dir("test-session");
        let ldir = log_dir("test-session");
        assert!(
            ldir.starts_with(&sdir),
            "log_dir '{}' should be under state_dir '{}'",
            ldir.display(),
            sdir.display()
        );
    }
}
