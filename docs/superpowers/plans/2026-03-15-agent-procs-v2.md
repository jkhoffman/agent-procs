# agent-procs v2 Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add process groups, fix socket path limits, move env vars into the protocol, and implement `--follow` streaming.

**Architecture:** Four independent changes to the existing daemon+CLI architecture. Each change is self-contained and can be tested independently. Changes 1-3 are small correctness fixes; Change 4 is the main feature addition.

**Tech Stack:** Rust, tokio, nix (process groups), clap, serde

**Spec:** `docs/superpowers/specs/2026-03-15-agent-procs-v2-design.md`

---

## File Changes Overview

All changes modify existing files. No new files created except tests.

| Change | Files Modified | Files Created |
|--------|---------------|---------------|
| 1. Process Groups | `src/daemon/process_manager.rs` | `tests/test_process_groups.rs` |
| 2. Socket Paths | `src/paths.rs`, `src/daemon/spawn.rs`, `src/cli/session_cmd.rs` | - (update existing tests) |
| 3. Protocol Env | `src/protocol.rs`, `src/daemon/process_manager.rs`, `src/cli/up.rs`, `src/cli/run.rs` | - (update existing tests) |
| 4. Follow Streaming | `src/protocol.rs`, `src/main.rs`, `src/daemon/server.rs`, `src/cli/logs.rs`, `src/cli/mod.rs` | `tests/test_follow.rs` |

---

## Chunk 1: Process Groups, Socket Paths, and Protocol Env

### Task 1: Process groups for child processes

**Files:**
- Modify: `src/daemon/process_manager.rs`
- Create: `tests/test_process_groups.rs`

- [ ] **Step 1: Write integration test for process group cleanup**

Create `tests/test_process_groups.rs`:

```rust
mod helpers;
use assert_cmd::Command;
use helpers::TestContext;
use std::thread;
use std::time::Duration;

/// Verify that stopping a process kills child processes too (not just the shell).
/// We spawn a shell that launches a subprocess, then stop it and check the subprocess is also gone.
#[test]
fn test_stop_kills_child_processes() {
    let ctx = TestContext::new("t-pgid");
    ctx.set_env();

    // Spawn a process that starts a subprocess which writes its PID to a file
    let pid_file = ctx.state_dir.path().join("child.pid");
    let pid_file_str = pid_file.to_str().unwrap();
    let cmd = format!(
        "bash -c 'echo $$ > {} && sleep 60' &; sleep 60",
        pid_file_str
    );

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run", &cmd, "--name", "parent"])
        .output().unwrap();
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
    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop", "parent"])
        .output().unwrap();
    assert!(output.status.success());

    // Wait a moment for signals to propagate
    thread::sleep(Duration::from_millis(500));

    // Verify child is dead
    assert!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(child_pid), None).is_err(),
        "child process should be dead after stop"
    );

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test test_process_groups -- --test-threads=1`
Expected: FAIL — child process survives because we signal only the shell PID, not the group

- [ ] **Step 3: Add process group setup to spawn**

In `src/daemon/process_manager.rs`, add the `pre_exec` hook and `CommandExt` import. Replace the spawn section (lines 52-62):

```rust
// Add to imports at top of file:
use std::os::unix::process::CommandExt;

// Replace the cmd setup (lines 52-56) with:
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        // Put child in its own process group so we can signal the entire tree
        unsafe {
            cmd.pre_exec(|| {
                nix::unistd::setpgid(nix::unistd::Pid::from_raw(0), nix::unistd::Pid::from_raw(0))
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                Ok(())
            });
        }
```

- [ ] **Step 4: Change stop to use killpg**

In `src/daemon/process_manager.rs`, replace the SIGTERM section in `stop_process()` (lines 98-104):

```rust
        if let Some(ref child) = proc.child {
            let raw_pid = child.id().unwrap_or(0) as i32;
            if raw_pid > 0 {
                // Signal the entire process group (child PID == PGID due to setpgid in pre_exec)
                let pgid = nix::unistd::Pid::from_raw(raw_pid);
                let _ = nix::sys::signal::killpg(pgid, nix::sys::signal::Signal::SIGTERM);
            }
        }
```

And replace the SIGKILL escalation (lines 118-123):

```rust
                _ => {
                    // Timed out or error — force kill the process group
                    let raw_pid = proc.pid as i32;
                    if raw_pid > 0 {
                        let pgid = nix::unistd::Pid::from_raw(raw_pid);
                        let _ = nix::sys::signal::killpg(pgid, nix::sys::signal::Signal::SIGKILL);
                    }
                    let _ = child.wait().await;
                    proc.exit_code = Some(-9);
                }
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --test test_process_groups -- --test-threads=1`
Expected: PASS

- [ ] **Step 6: Run all tests**

Run: `cargo test -- --test-threads=1`
Expected: All tests pass (existing tests should still work since process groups are transparent)

- [ ] **Step 7: Commit**

```bash
git add src/daemon/process_manager.rs tests/test_process_groups.rs
git commit -m "feat: spawn children in process groups, signal entire tree on stop"
```

---

### Task 2: Socket path fix

**Files:**
- Modify: `src/paths.rs`
- Modify: `src/daemon/spawn.rs`
- Modify: `src/cli/session_cmd.rs`
- Modify: `tests/test_paths.rs`

- [ ] **Step 1: Update paths.rs**

Replace `src/paths.rs` entirely:

```rust
use std::env;
use std::path::PathBuf;

/// Base directory for sockets and PID files: /tmp/agent-procs-<uid>/
/// Short fixed path to avoid macOS 103-byte socket path limit.
pub fn socket_base_dir() -> PathBuf {
    let uid = nix::unistd::getuid();
    PathBuf::from(format!("/tmp/agent-procs-{}", uid))
}

pub fn socket_path(session: &str) -> PathBuf {
    socket_base_dir().join(format!("{}.sock", session))
}

pub fn pid_path(session: &str) -> PathBuf {
    socket_base_dir().join(format!("{}.pid", session))
}

/// State directory for persistent data (logs, state.json).
/// Uses $XDG_STATE_HOME, defaults to ~/.local/state/.
pub fn state_dir(session: &str) -> PathBuf {
    let base = match env::var("XDG_STATE_HOME") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => {
            let home = env::var("HOME").expect("HOME not set");
            PathBuf::from(home).join(".local/state")
        }
    };
    base.join("agent-procs/sessions").join(session)
}

pub fn log_dir(session: &str) -> PathBuf { state_dir(session).join("logs") }
pub fn state_file(session: &str) -> PathBuf { state_dir(session).join("state.json") }
```

- [ ] **Step 2: Update spawn.rs to create socket dir with 0700**

In `src/daemon/spawn.rs`, in `spawn_daemon()`, replace the runtime dir creation (line 10-13):

```rust
    // Create socket base dir with restricted permissions
    let socket_dir = paths::socket_base_dir();
    fs::create_dir_all(&socket_dir)?;
    // Set permissions to 0700 (owner only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&socket_dir, fs::Permissions::from_mode(0o700))?;
    }

    let state = paths::state_dir(session);
    fs::create_dir_all(state.join("logs"))?;
```

Do the same in `run_daemon()` (lines 61-64):

```rust
    let socket_dir = paths::socket_base_dir();
    let _ = std::fs::create_dir_all(&socket_dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&socket_dir, std::fs::Permissions::from_mode(0o700));
    }
    let state = paths::state_dir(session);
    let _ = std::fs::create_dir_all(state.join("logs"));
```

- [ ] **Step 3: Update session_cmd.rs to scan flat directory**

Replace `src/cli/session_cmd.rs`:

```rust
use crate::paths;
use crate::session;

pub async fn list() -> i32 {
    let base = paths::socket_base_dir();

    let entries = match std::fs::read_dir(&base) {
        Ok(e) => e,
        Err(_) => {
            println!("no active sessions");
            return 0;
        }
    };

    println!("{:<20} STATUS", "SESSION");
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".pid") { continue; }
        let session_name = name.trim_end_matches(".pid");
        let status = if session::is_daemon_alive(&entry.path()) {
            "running"
        } else {
            "stale"
        };
        println!("{:<20} {}", session_name, status);
    }
    0
}

pub async fn clean() -> i32 {
    let base = paths::socket_base_dir();

    let entries = match std::fs::read_dir(&base) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".pid") { continue; }
        let session_name = name.trim_end_matches(".pid");

        if !session::is_daemon_alive(&entry.path()) {
            // Remove socket and PID files
            let _ = std::fs::remove_file(paths::socket_path(session_name));
            let _ = std::fs::remove_file(paths::pid_path(session_name));
            // Remove XDG state directory (logs, state.json)
            let _ = std::fs::remove_dir_all(paths::state_dir(session_name));
            println!("cleaned stale session: {}", session_name);
        }
    }
    0
}
```

- [ ] **Step 4: Update test_paths.rs**

Replace `tests/test_paths.rs` — remove runtime_dir tests, add socket_base_dir tests:

```rust
use std::env;
use tempfile::TempDir;

#[test]
fn test_socket_base_dir_uses_tmp() {
    let result = agent_procs::paths::socket_base_dir();
    let uid = nix::unistd::getuid();
    assert_eq!(result, std::path::PathBuf::from(format!("/tmp/agent-procs-{}", uid)));
}

#[test]
fn test_socket_path() {
    let result = agent_procs::paths::socket_path("mysession");
    let uid = nix::unistd::getuid();
    assert_eq!(result, std::path::PathBuf::from(format!("/tmp/agent-procs-{}/mysession.sock", uid)));
}

#[test]
fn test_pid_path() {
    let result = agent_procs::paths::pid_path("mysession");
    let uid = nix::unistd::getuid();
    assert_eq!(result, std::path::PathBuf::from(format!("/tmp/agent-procs-{}/mysession.pid", uid)));
}

#[test]
fn test_state_dir_uses_xdg_when_set() {
    let tmp = TempDir::new().unwrap();
    env::set_var("XDG_STATE_HOME", tmp.path());
    let result = agent_procs::paths::state_dir("test-session");
    assert_eq!(result, tmp.path().join("agent-procs/sessions/test-session"));
    env::remove_var("XDG_STATE_HOME");
}

#[test]
fn test_state_dir_defaults_to_home() {
    env::remove_var("XDG_STATE_HOME");
    let result = agent_procs::paths::state_dir("test-session");
    let home = env::var("HOME").unwrap();
    let expected = std::path::PathBuf::from(format!(
        "{}/.local/state/agent-procs/sessions/test-session", home
    ));
    assert_eq!(result, expected);
}

#[test]
fn test_log_dir() {
    let tmp = TempDir::new().unwrap();
    env::set_var("XDG_STATE_HOME", tmp.path());
    let result = agent_procs::paths::log_dir("mysession");
    assert_eq!(result, tmp.path().join("agent-procs/sessions/mysession/logs"));
    env::remove_var("XDG_STATE_HOME");
}

#[test]
fn test_socket_path_is_short() {
    // Verify socket paths stay well under macOS 103-byte limit
    let path = agent_procs::paths::socket_path("my-long-session-name-that-would-have-failed-before");
    assert!(path.to_string_lossy().len() < 100, "socket path too long: {}", path.display());
}
```

- [ ] **Step 5: Update tests/helpers/mod.rs**

Remove the `runtime_dir` field (sockets now use fixed `/tmp` path, not a temp dir). Stop setting `XDG_RUNTIME_DIR`. Keep `XDG_STATE_HOME` for log isolation:

```rust
use tempfile::TempDir;

pub struct TestContext {
    pub state_dir: TempDir,
    pub session: String,
}

impl TestContext {
    pub fn new(session: &str) -> Self {
        Self {
            state_dir: TempDir::new().unwrap(),
            session: session.into(),
        }
    }

    pub fn set_env(&self) {
        std::env::set_var("XDG_STATE_HOME", self.state_dir.path());
    }
}
```

- [ ] **Step 6: Update tests/test_session_mgmt.rs**

The `test_session_clean_removes_stale` test needs to write the fake PID to the actual socket base dir path, not the old XDG runtime dir:

```rust
#[test]
fn test_session_clean_removes_stale() {
    let ctx = TestContext::new("t-sess-cln");
    ctx.set_env();

    // Start and stop to create a session
    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run", "sleep 60", "--name", "bg"])
        .output().unwrap();
    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();

    // Write a fake PID to make it look stale
    let pid_path = agent_procs::paths::pid_path(&ctx.session);
    if pid_path.exists() {
        std::fs::write(&pid_path, "99999999\n").unwrap();
    }

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["session", "clean"])
        .output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cleaned"));
}
```

- [ ] **Step 7: Run all tests**

Run: `cargo test -- --test-threads=1`
Expected: All tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/paths.rs src/daemon/spawn.rs src/cli/session_cmd.rs tests/test_paths.rs
git commit -m "fix: move sockets to /tmp to avoid macOS path length limit"
```

---

### Task 3: Protocol-level env vars

**Files:**
- Modify: `src/protocol.rs`
- Modify: `src/daemon/process_manager.rs`
- Modify: `src/cli/up.rs`
- Modify: `src/cli/run.rs`
- Modify: `tests/test_protocol.rs`
- Modify: `tests/test_up_down.rs`

- [ ] **Step 1: Add `env` to `Request::Run` in protocol.rs**

In `src/protocol.rs`, add the import and update `Request::Run`:

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    Run {
        command: String,
        name: Option<String>,
        cwd: Option<String>,
        #[serde(default)]
        env: Option<HashMap<String, String>>,
    },
    // ... rest unchanged
```

- [ ] **Step 2: Handle env in process_manager.rs**

In `src/daemon/process_manager.rs`, update `spawn_process` signature and add env handling:

```rust
    pub async fn spawn_process(&mut self, command: &str, name: Option<String>, cwd: Option<&str>, env: Option<&HashMap<String, String>>) -> Response {
```

After the `current_dir` call, add:

```rust
        if let Some(env_vars) = env {
            cmd.envs(env_vars);
        }
```

Update `restart_process` to pass `None` for env:

```rust
        self.spawn_process(&command, Some(name), None, None).await
```

- [ ] **Step 3: Update server.rs to pass env through**

In `src/daemon/server.rs`, update the `Request::Run` handler:

```rust
        Request::Run { command, name, cwd, env } => {
            state.lock().await.process_manager.spawn_process(&command, name, cwd.as_deref(), env.as_ref()).await
        }
```

- [ ] **Step 4: Update cli/run.rs**

```rust
    let req = Request::Run { command: command.into(), name, cwd: None, env: None };
```

- [ ] **Step 5: Update cli/up.rs — remove shell escaping, pass env through protocol**

Replace the env handling and run request in `src/cli/up.rs`:

```rust
            let def = &config.processes[name];

            // Resolve cwd relative to config file directory
            let resolved_cwd = def.cwd.as_ref().map(|c| {
                let p = std::path::Path::new(c);
                if p.is_relative() {
                    path.parent().unwrap_or(std::path::Path::new(".")).join(p).to_string_lossy().to_string()
                } else {
                    c.clone()
                }
            });

            // Pass env vars through the protocol (no shell escaping needed)
            let env = if def.env.is_empty() { None } else { Some(def.env.clone()) };

            // Start the process
            let req = Request::Run {
                command: def.cmd.clone(),
                name: Some(name.clone()),
                cwd: resolved_cwd,
                env,
            };
```

Remove the `shell_escape()` function at the bottom of the file.

- [ ] **Step 6: Update test_protocol.rs roundtrip test**

Update the `test_run_request_roundtrip` test:

```rust
#[test]
fn test_run_request_roundtrip() {
    let req = Request::Run {
        command: "npm run dev".into(),
        name: Some("webserver".into()),
        cwd: None,
        env: Some(std::collections::HashMap::from([
            ("NODE_ENV".to_string(), "production".to_string()),
        ])),
    };
    let json = serde_json::to_string(&req).unwrap();
    let decoded: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, decoded);
}
```

- [ ] **Step 7: Run all tests**

Run: `cargo test -- --test-threads=1`
Expected: All tests pass — the `test_up_with_env_and_cwd` test should still pass since env vars now go through the protocol properly.

- [ ] **Step 8: Commit**

```bash
git add src/protocol.rs src/daemon/process_manager.rs src/daemon/server.rs src/cli/run.rs src/cli/up.rs tests/test_protocol.rs
git commit -m "feat: pass env vars through protocol instead of shell export prefix"
```

---

## Chunk 2: `--follow` Streaming

### Task 4: Add streaming reader helper to CLI

**Files:**
- Modify: `src/cli/mod.rs`

- [ ] **Step 1: Add `stream_responses` function to `src/cli/mod.rs`**

Add after the existing `request()` function:

```rust
use crate::protocol::Stream as ProtoStream;

/// Send a request and read streaming responses until LogEnd or error.
/// Calls `on_line` for each LogLine received. Returns the terminal response.
pub async fn stream_responses(
    session: &str,
    req: &Request,
    auto_spawn: bool,
    mut on_line: impl FnMut(&str, ProtoStream, &str),
) -> Result<Response, String> {
    let stream = connect(session, auto_spawn).await?;
    let (reader, mut writer) = stream.into_split();

    let mut json = serde_json::to_string(req).unwrap();
    json.push('\n');
    writer.write_all(json.as_bytes()).await.map_err(|e| format!("write error: {}", e))?;
    writer.flush().await.map_err(|e| format!("flush error: {}", e))?;

    let mut lines = BufReader::new(reader);
    loop {
        let mut line = String::new();
        let n = lines.read_line(&mut line).await.map_err(|e| format!("read error: {}", e))?;
        if n == 0 { return Ok(Response::LogEnd); } // EOF

        let resp: Response = serde_json::from_str(&line).map_err(|e| format!("parse error: {}", e))?;
        match resp {
            Response::LogLine { ref process, stream, ref line } => {
                on_line(process, stream, line);
            }
            Response::LogEnd | Response::Error { .. } => return Ok(resp),
            other => return Ok(other), // unexpected
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles (function is defined but not yet called)

- [ ] **Step 3: Commit**

```bash
git add src/cli/mod.rs
git commit -m "feat: add streaming response reader for --follow support"
```

---

### Task 5: Add `--lines` flag and protocol field

**Files:**
- Modify: `src/protocol.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Add `lines` to `Request::Logs` in protocol.rs**

Update the `Logs` variant:

```rust
    Logs {
        target: Option<String>,
        tail: usize,
        follow: bool,
        stderr: bool,
        all: bool,
        timeout_secs: Option<u64>,
        #[serde(default)]
        lines: Option<usize>,
    },
```

- [ ] **Step 2: Add `--lines` flag to main.rs**

In the `Logs` variant of `Commands`, add:

```rust
        /// Max lines for --follow
        #[arg(long)]
        lines: Option<usize>,
```

Update the `Logs` dispatch in `main()` to pass the new field:

```rust
        Commands::Logs { target, tail, follow, stderr, all, timeout, lines } => {
            agent_procs::cli::logs::execute(session, target.as_deref(), tail, follow, stderr, all, timeout, lines).await
        }
```

- [ ] **Step 3: Update logs.rs execute signature**

Add `lines: Option<usize>` to the `execute` function signature in `src/cli/logs.rs`:

```rust
pub async fn execute(
    session: &str, target: Option<&str>, tail: usize,
    follow: bool, stderr: bool, all: bool, timeout: Option<u64>, lines: Option<usize>,
) -> i32 {
```

(The `follow` implementation comes in the next task — for now just accept the parameter.)

- [ ] **Step 4: Verify it compiles**

Run: `cargo build`
Expected: Compiles

- [ ] **Step 5: Commit**

```bash
git add src/protocol.rs src/main.rs src/cli/logs.rs
git commit -m "feat: add --lines flag to logs command"
```

---

### Task 6: Implement daemon-side follow streaming

**Files:**
- Modify: `src/daemon/server.rs`

- [ ] **Step 1: Replace the Logs handler in server.rs**

Replace the `Request::Logs { .. }` match arm in `handle_request` with a new handler. Since follow streaming needs to write multiple responses, it can't use the single-response `handle_request` pattern. Instead, handle it directly in the connection loop.

Refactor `server.rs`: extract follow handling into a separate async function, and update the connection handler to detect follow requests and handle them specially.

In `handle_request`, change the `Logs` arm to only handle the non-follow case:

```rust
        Request::Logs { follow: false, .. } => {
            // Non-follow logs are read directly from disk by the CLI
            Response::Error { code: 1, message: "non-follow logs are read directly from disk by CLI".into() }
        }
        Request::Logs { follow: true, .. } => {
            // Handled separately in connection loop (needs streaming)
            Response::Error { code: 1, message: "follow requests handled in connection loop".into() }
        }
```

Then, in the connection handler's `while let` loop (after deserializing the request), add follow handling before calling `handle_request`:

```rust
                // Handle follow requests with streaming (before handle_request)
                if let Request::Logs { follow: true, ref target, all, timeout_secs, lines, .. } = request {
                    let output_rx = state.lock().await.process_manager.output_tx.subscribe();
                    let timeout = Duration::from_secs(timeout_secs.unwrap_or(30));
                    let max_lines = lines;
                    let target_filter = target.clone();
                    let show_all = all;

                    handle_follow_stream(
                        &writer, output_rx, target_filter, show_all, timeout, max_lines
                    ).await;
                    continue; // Don't call handle_request
                }
```

Add the `handle_follow_stream` function:

```rust
async fn handle_follow_stream(
    writer: &Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
    mut output_rx: broadcast::Receiver<super::log_writer::OutputLine>,
    target: Option<String>,
    all: bool,
    timeout: Duration,
    max_lines: Option<usize>,
) {
    let mut line_count: usize = 0;

    let result = tokio::time::timeout(timeout, async {
        loop {
            match output_rx.recv().await {
                Ok(output_line) => {
                    // Filter by target (unless --all)
                    if !all {
                        if let Some(ref t) = target {
                            if output_line.process != *t { continue; }
                        }
                    }

                    let resp = Response::LogLine {
                        process: output_line.process,
                        stream: output_line.stream,
                        line: output_line.line,
                    };
                    if send_response(writer, &resp).await.is_err() {
                        return; // Client disconnected
                    }

                    line_count += 1;
                    if let Some(max) = max_lines {
                        if line_count >= max { return; }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }).await;

    // Send LogEnd (whether from timeout, line limit, or channel close)
    let _ = send_response(writer, &Response::LogEnd).await;
}
```

Add the import at the top of server.rs:

```rust
use tokio::sync::broadcast;
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add src/daemon/server.rs
git commit -m "feat: add daemon-side follow streaming handler"
```

---

### Task 7: Implement CLI-side follow and integration tests

**Files:**
- Modify: `src/cli/logs.rs`
- Create: `tests/test_follow.rs`

- [ ] **Step 1: Write integration test for --follow**

Create `tests/test_follow.rs`:

```rust
mod helpers;
use assert_cmd::Command;
use helpers::TestContext;
use std::time::Duration;

#[test]
fn test_follow_captures_output() {
    let ctx = TestContext::new("t-follow");
    ctx.set_env();

    // Start a process that waits 1s then outputs lines (gives --follow time to subscribe)
    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run",
            "sleep 1 && for i in 1 2 3 4 5; do echo line-$i; sleep 0.2; done",
            "--name", "counter"])
        .output().unwrap();

    // Follow with a line limit
    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "logs", "counter",
            "--follow", "--lines", "3", "--timeout", "10"])
        .timeout(Duration::from_secs(15))
        .output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("line-1"));
    assert!(stdout.contains("line-2"));
    assert!(stdout.contains("line-3"));
    // Should have stopped after 3 lines
    let line_count = stdout.lines().count();
    assert_eq!(line_count, 3, "expected 3 lines, got {}: {}", line_count, stdout);

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}

#[test]
fn test_follow_timeout() {
    let ctx = TestContext::new("t-fol-tmo");
    ctx.set_env();

    // Start a process that outputs nothing after initial line
    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run",
            "echo started && sleep 60",
            "--name", "quiet"])
        .output().unwrap();

    // Follow with short timeout — should get the initial line then timeout
    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "logs", "quiet",
            "--follow", "--timeout", "2"])
        .timeout(Duration::from_secs(10))
        .output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("started"));

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}

#[test]
fn test_follow_all_processes() {
    let ctx = TestContext::new("t-fol-all");
    ctx.set_env();

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run",
            "echo alpha-out && sleep 60", "--name", "alpha"])
        .output().unwrap();
    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run",
            "echo beta-out && sleep 60", "--name", "beta"])
        .output().unwrap();

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "logs",
            "--all", "--follow", "--lines", "2", "--timeout", "10"])
        .timeout(Duration::from_secs(15))
        .output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[alpha]") || stdout.contains("alpha-out"));
    assert!(stdout.contains("[beta]") || stdout.contains("beta-out"));

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}
```

- [ ] **Step 2: Implement follow mode in cli/logs.rs**

Replace `src/cli/logs.rs`:

```rust
use crate::paths;
use crate::protocol::{Request, Response, Stream as ProtoStream};
use std::io::{BufRead, BufReader};
use std::fs::File;

pub async fn execute(
    session: &str, target: Option<&str>, tail: usize,
    follow: bool, stderr: bool, all: bool, timeout: Option<u64>, lines: Option<usize>,
) -> i32 {
    if follow {
        return execute_follow(session, target, all, timeout, lines).await;
    }

    // Non-follow: read from disk (unchanged)
    let log_dir = paths::log_dir(session);

    if all || target.is_none() {
        return show_all_logs(&log_dir, tail);
    }

    let target = target.unwrap();
    let stream = if stderr { "stderr" } else { "stdout" };
    let path = log_dir.join(format!("{}.{}", target, stream));

    match tail_file(&path, tail) {
        Ok(lines) => { for line in lines { println!("{}", line); } 0 }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("error: no logs for process '{}' ({})", target, stream);
            2
        }
        Err(e) => { eprintln!("error reading logs: {}", e); 1 }
    }
}

async fn execute_follow(
    session: &str, target: Option<&str>, all: bool, timeout: Option<u64>, lines: Option<usize>,
) -> i32 {
    let req = Request::Logs {
        target: target.map(|t| t.to_string()),
        tail: 0,
        follow: true,
        stderr: false,
        all: all || target.is_none(),
        timeout_secs: timeout,
        lines,
    };

    match crate::cli::stream_responses(session, &req, false, |process, _stream, line| {
        if all || target.is_none() {
            println!("[{}] {}", process, line);
        } else {
            println!("{}", line);
        }
    }).await {
        Ok(Response::LogEnd) => 0,
        Ok(Response::Error { code, message }) => { eprintln!("error: {}", message); code }
        Ok(_) => 0,
        Err(e) => { eprintln!("error: {}", e); 1 }
    }
}

fn show_all_logs(log_dir: &std::path::Path, tail: usize) -> i32 {
    let entries = match std::fs::read_dir(log_dir) {
        Ok(e) => e,
        Err(e) => { eprintln!("error: cannot read log dir: {}", e); return 1; }
    };

    let mut all_lines: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".stdout") { continue; }
        let proc_name = name.trim_end_matches(".stdout").to_string();
        if let Ok(lines) = tail_file(&entry.path(), tail) {
            for line in lines {
                all_lines.push((proc_name.to_string(), line));
            }
        }
    }

    for (proc_name, line) in &all_lines {
        println!("[{}] {}", proc_name, line);
    }
    0
}

fn tail_file(path: &std::path::Path, n: usize) -> std::io::Result<Vec<String>> {
    let file = File::open(path)?;
    let mut ring: std::collections::VecDeque<String> = std::collections::VecDeque::with_capacity(n);
    for line in BufReader::new(file).lines() {
        let line = line?;
        if ring.len() == n {
            ring.pop_front();
        }
        ring.push_back(line);
    }
    Ok(ring.into_iter().collect())
}
```

- [ ] **Step 3: Run follow tests**

Run: `cargo test --test test_follow -- --test-threads=1`
Expected: All 3 tests pass

- [ ] **Step 4: Run all tests**

Run: `cargo test -- --test-threads=1`
Expected: All tests pass

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: Clean

- [ ] **Step 6: Commit**

```bash
git add src/cli/logs.rs tests/test_follow.rs
git commit -m "feat: implement --follow streaming with --lines and --timeout"
```

---

### Task 8: Update skill document

**Files:**
- Modify: `skill/agent-procs.md`

- [ ] **Step 1: Add --follow documentation to the skill**

In `skill/agent-procs.md`, update the "Checking output" section to include `--follow`:

```markdown
## Checking output

```bash
agent-procs logs webserver --tail 50      # last 50 lines
agent-procs logs webserver --stderr       # stderr only
agent-procs logs --all --tail 30          # all processes interleaved

# Real-time streaming
agent-procs logs webserver --follow --timeout 30    # stream for 30s
agent-procs logs webserver --follow --lines 20      # stream until 20 lines
agent-procs logs --all --follow --timeout 30        # all processes live
```
```

- [ ] **Step 2: Commit**

```bash
git add skill/agent-procs.md
git commit -m "docs: add --follow to skill document"
```

---

### Task 9: Final verification

- [ ] **Step 1: Run full test suite**

Run: `cargo test -- --test-threads=1`
Expected: All tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: Clean

- [ ] **Step 3: Build release**

Run: `cargo build --release`
Expected: Builds successfully

- [ ] **Step 4: Commit any remaining fixes**

Stage only files modified for fixes, then commit.
