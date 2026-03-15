# agent-procs Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a concurrent process runner CLI for AI agents — headless daemon + thin CLI client with process lifecycle management, output capture, pattern-based waiting, and config-file-driven startup.

**Architecture:** A background daemon (auto-spawned on first use) owns all child processes and communicates with CLI clients over a Unix domain socket using newline-delimited JSON. Child output is piped through the daemon (enabling pattern matching for the wait engine) and persisted to disk. An optional TUI is out of scope for v1.

**Tech Stack:** Rust, tokio (async runtime), clap (CLI), serde/serde_yaml (serialization), nix (Unix process control)

**Spec:** `docs/superpowers/specs/2026-03-15-agent-procs-design.md`

---

## File Structure

```
agent-procs/
  Cargo.toml
  src/
    main.rs                  # CLI entry point, clap definitions, dispatch
    lib.rs                   # Library root, pub mod declarations
    paths.rs                 # XDG directory resolution
    session.rs               # Session ID derivation, stale detection
    protocol.rs              # Shared types: Request, Response, ProcessInfo
    config.rs                # agent-procs.yaml parsing and discovery
    daemon/
      mod.rs                 # Daemon entry: pub mod declarations
      spawn.rs               # Double-fork to background the daemon
      process_manager.rs     # Child spawn, signal, reap, state tracking
      log_writer.rs          # Pipe stdout/stderr to log files, rotation, broadcast
      wait_engine.rs         # Pattern subscriptions, match + notify
      server.rs              # Unix socket listener, request routing
    cli/
      mod.rs                 # Shared client helpers (connect to socket, send/recv)
      run.rs                 # `run` command
      stop.rs                # `stop` and `stop-all` commands
      restart.rs             # `restart` command
      status.rs              # `status` command
      logs.rs                # `logs` command (--tail, --stderr, --all)
      wait.rs                # `wait` command (--until, --exit)
      up.rs                  # `up` command (config-based startup)
      down.rs                # `down` command
      session_cmd.rs         # `session list` and `session clean`
  tests/
    helpers/
      mod.rs                 # Test utilities: start daemon, run CLI, temp dirs
    test_paths.rs            # Unit tests for XDG resolution
    test_protocol.rs         # Unit tests for serialization round-trips
    test_config.rs           # Unit tests for config parsing
    test_session.rs          # Unit tests for session ID, stale detection
    test_run_stop.rs         # Integration: run, status, stop
    test_logs.rs             # Integration: logs --tail, --stderr, --all
    test_wait.rs             # Integration: wait --until, wait --exit
    test_up_down.rs          # Integration: config-based up/down
    test_session_mgmt.rs     # Integration: session list, session clean
    test_log_rotation.rs     # Integration: log size limits
  skill/
    agent-procs.md           # Skill document for LLM agents
```

**Design notes:**
- `protocol.rs` is the shared contract between daemon and CLI — both depend on it, neither depends on the other.
- `daemon/` modules are internal to the daemon process. `cli/` modules are internal to the CLI process. They communicate only through `protocol.rs` types over the socket.
- Each daemon sub-module has one responsibility: `process_manager` owns children, `log_writer` captures output to disk and broadcasts lines, `wait_engine` handles pattern subscriptions, `server` handles socket I/O.
- Child stdout/stderr is piped through the daemon (not redirected to files directly) so the wait engine can scan lines in real-time.
- Integration tests spawn a real daemon and interact via the CLI binary. Tests that mutate env vars must run with `--test-threads=1`.

---

## Chunk 1: Project Scaffolding, Protocol, and Paths

### Task 1: Initialize Rust project

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`

- [ ] **Step 1: Create the Rust project**

```bash
cd /Users/jkh/src/tries/2026-03-15-agent-procs
cargo init --name agent-procs
```

- [ ] **Step 2: Add dependencies to Cargo.toml**

```toml
[package]
name = "agent-procs"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "agent-procs"
path = "src/main.rs"

[lib]
name = "agent_procs"
path = "src/lib.rs"

[dependencies]
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
tokio = { version = "1", features = ["full"] }
nix = { version = "0.29", features = ["signal", "process", "user"] }
thiserror = "2"
regex = "1"

[dev-dependencies]
tempfile = "3"
assert_cmd = "2"
predicates = "3"
```

- [ ] **Step 3: Create lib.rs**

Create `src/lib.rs`:

```rust
pub mod paths;
// Uncomment each module as it is created in subsequent tasks:
// pub mod protocol;
// pub mod config;
// pub mod session;
// pub mod daemon;
// pub mod cli;
```

- [ ] **Step 4: Set up main.rs with clap skeleton**

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "agent-procs", about = "Concurrent process runner for AI agents")]
struct Cli {
    /// Session name (default: "default")
    #[arg(long, global = true, default_value = "default")]
    session: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Spawn a new process
    Run {
        /// Command to execute
        command: String,
        /// Process name (auto-generated if omitted)
        #[arg(long)]
        name: Option<String>,
    },
    /// Stop a process
    Stop { target: String },
    /// Stop all processes
    StopAll,
    /// Restart a process
    Restart { target: String },
    /// Show status of all processes
    Status {
        #[arg(long)]
        json: bool,
    },
    /// View process logs
    Logs {
        target: Option<String>,
        #[arg(long, default_value = "100")]
        tail: usize,
        #[arg(long)]
        follow: bool,
        #[arg(long)]
        stderr: bool,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Wait for a process condition
    Wait {
        target: String,
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        regex: bool,
        #[arg(long)]
        exit: bool,
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Start all processes from config
    Up {
        #[arg(long)]
        only: Option<String>,
        #[arg(long)]
        config: Option<String>,
    },
    /// Stop all config-managed processes
    Down,
    /// Session management
    Session {
        #[command(subcommand)]
        command: SessionCommands,
    },
}

#[derive(Subcommand)]
enum SessionCommands {
    List,
    Clean,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    // Commands will be wired up in later tasks
    eprintln!("agent-procs: command not yet implemented");
    std::process::exit(1);
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo build`
Expected: Compiles (lib.rs modules commented out)

- [ ] **Step 6: Verify CLI help works**

Run: `cargo run -- --help`
Expected: Shows help with all subcommands listed

- [ ] **Step 7: Commit**

```bash
git init
echo '/target' > .gitignore
echo '/.superpowers' >> .gitignore
git add Cargo.toml Cargo.lock src/main.rs src/lib.rs .gitignore
git commit -m "feat: initialize project with clap CLI skeleton"
```

---

### Task 2: XDG path resolution

**Files:**
- Create: `src/paths.rs`
- Create: `tests/test_paths.rs`
- Modify: `src/lib.rs` (uncomment `pub mod paths;`)

- [ ] **Step 1: Write failing tests for path resolution**

Create `tests/test_paths.rs`:

```rust
use std::env;
use tempfile::TempDir;

// These tests mutate process-wide env vars. MUST run with --test-threads=1.

#[test]
fn test_runtime_dir_uses_xdg_when_set() {
    let tmp = TempDir::new().unwrap();
    env::set_var("XDG_RUNTIME_DIR", tmp.path());
    let result = agent_procs::paths::runtime_dir("test-session");
    assert_eq!(result, tmp.path().join("agent-procs/sessions/test-session"));
    env::remove_var("XDG_RUNTIME_DIR");
}

#[test]
fn test_runtime_dir_falls_back_to_tmp() {
    env::remove_var("XDG_RUNTIME_DIR");
    let result = agent_procs::paths::runtime_dir("test-session");
    let uid = nix::unistd::getuid();
    let expected = std::path::PathBuf::from(format!(
        "/tmp/agent-procs-{}/sessions/test-session",
        uid
    ));
    assert_eq!(result, expected);
}

#[test]
fn test_sessions_base_dir() {
    let tmp = TempDir::new().unwrap();
    env::set_var("XDG_RUNTIME_DIR", tmp.path());
    let result = agent_procs::paths::sessions_base_dir();
    assert_eq!(result, tmp.path().join("agent-procs/sessions"));
    env::remove_var("XDG_RUNTIME_DIR");
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
fn test_socket_path() {
    let tmp = TempDir::new().unwrap();
    env::set_var("XDG_RUNTIME_DIR", tmp.path());
    let result = agent_procs::paths::socket_path("mysession");
    assert_eq!(result, tmp.path().join("agent-procs/sessions/mysession/socket"));
    env::remove_var("XDG_RUNTIME_DIR");
}

#[test]
fn test_pid_path() {
    let tmp = TempDir::new().unwrap();
    env::set_var("XDG_RUNTIME_DIR", tmp.path());
    let result = agent_procs::paths::pid_path("mysession");
    assert_eq!(result, tmp.path().join("agent-procs/sessions/mysession/daemon.pid"));
    env::remove_var("XDG_RUNTIME_DIR");
}

#[test]
fn test_log_dir() {
    let tmp = TempDir::new().unwrap();
    env::set_var("XDG_STATE_HOME", tmp.path());
    let result = agent_procs::paths::log_dir("mysession");
    assert_eq!(result, tmp.path().join("agent-procs/sessions/mysession/logs"));
    env::remove_var("XDG_STATE_HOME");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test test_paths -- --test-threads=1`
Expected: Compilation fails — `paths` module doesn't exist yet

- [ ] **Step 3: Implement paths module**

Create `src/paths.rs`:

```rust
use std::env;
use std::path::PathBuf;

pub fn runtime_dir(session: &str) -> PathBuf {
    match env::var("XDG_RUNTIME_DIR") {
        Ok(dir) => PathBuf::from(dir).join("agent-procs/sessions").join(session),
        Err(_) => {
            let uid = nix::unistd::getuid();
            PathBuf::from(format!("/tmp/agent-procs-{}", uid))
                .join("sessions")
                .join(session)
        }
    }
}

/// Base directory containing all sessions (for listing/cleaning).
pub fn sessions_base_dir() -> PathBuf {
    match env::var("XDG_RUNTIME_DIR") {
        Ok(dir) => PathBuf::from(dir).join("agent-procs/sessions"),
        Err(_) => {
            let uid = nix::unistd::getuid();
            PathBuf::from(format!("/tmp/agent-procs-{}", uid)).join("sessions")
        }
    }
}

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

pub fn socket_path(session: &str) -> PathBuf {
    runtime_dir(session).join("socket")
}

pub fn pid_path(session: &str) -> PathBuf {
    runtime_dir(session).join("daemon.pid")
}

pub fn log_dir(session: &str) -> PathBuf {
    state_dir(session).join("logs")
}

pub fn state_file(session: &str) -> PathBuf {
    state_dir(session).join("state.json")
}
```

- [ ] **Step 4: Update lib.rs — uncomment `pub mod paths;`**

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --test test_paths -- --test-threads=1`
Expected: All 7 tests pass

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/paths.rs tests/test_paths.rs
git commit -m "feat: add XDG-compliant path resolution"
```

---

### Task 3: Protocol types

**Files:**
- Create: `src/protocol.rs`
- Create: `tests/test_protocol.rs`
- Modify: `src/lib.rs` (uncomment `pub mod protocol;`)

- [ ] **Step 1: Write failing tests for protocol serialization**

Create `tests/test_protocol.rs`:

```rust
use agent_procs::protocol::*;

#[test]
fn test_run_request_roundtrip() {
    let req = Request::Run { command: "npm run dev".into(), name: Some("webserver".into()), cwd: None };
    let json = serde_json::to_string(&req).unwrap();
    let decoded: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn test_status_response_roundtrip() {
    let resp = Response::Status {
        processes: vec![ProcessInfo {
            name: "webserver".into(), id: "p1".into(), pid: 12345,
            state: ProcessState::Running, exit_code: None,
            uptime_secs: Some(150), command: "npm run dev".into(),
        }],
    };
    let json = serde_json::to_string(&resp).unwrap();
    let decoded: Response = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, decoded);
}

#[test]
fn test_wait_request_with_pattern() {
    let req = Request::Wait {
        target: "webserver".into(), until: Some("Listening on".into()),
        regex: false, exit: false, timeout_secs: Some(30),
    };
    let json = serde_json::to_string(&req).unwrap();
    let decoded: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn test_error_response() {
    let resp = Response::Error { code: 2, message: "process not found: foo".into() };
    let json = serde_json::to_string(&resp).unwrap();
    let decoded: Response = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, decoded);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test test_protocol`
Expected: Compilation fails

- [ ] **Step 3: Implement protocol types**

Create `src/protocol.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    Run { command: String, name: Option<String>, cwd: Option<String> },
    Stop { target: String },
    StopAll,
    Restart { target: String },
    Status,
    Logs { target: Option<String>, tail: usize, follow: bool, stderr: bool, all: bool, timeout_secs: Option<u64> },
    Wait { target: String, until: Option<String>, regex: bool, exit: bool, timeout_secs: Option<u64> },
    Shutdown,
}

// Note: Up/Down are not protocol messages — they decompose client-side into Run/Wait/StopAll.

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    Ok { message: String },
    RunOk { name: String, id: String, pid: u32 },
    Status { processes: Vec<ProcessInfo> },
    LogLine { process: String, stream: Stream, line: String },
    LogEnd,
    WaitMatch { line: String },
    WaitExited { exit_code: i32 },
    WaitTimeout,
    Error { code: i32, message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub name: String,
    pub id: String,
    pub pid: u32,
    pub state: ProcessState,
    pub exit_code: Option<i32>,
    pub uptime_secs: Option<u64>,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessState { Running, Exited }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream { Stdout, Stderr }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test test_protocol`
Expected: All 4 tests pass

- [ ] **Step 5: Commit**

```bash
git add src/protocol.rs tests/test_protocol.rs src/lib.rs
git commit -m "feat: add protocol types for daemon-CLI communication"
```

---

### Task 4: Config file parsing

**Files:**
- Create: `src/config.rs`
- Create: `tests/test_config.rs`

- [ ] **Step 1: Write failing tests**

Create `tests/test_config.rs`:

```rust
use agent_procs::config::*;
use std::io::Write;
use tempfile::TempDir;

#[test]
fn test_parse_minimal_config() {
    let yaml = "processes:\n  web:\n    cmd: npm run dev\n";
    let config: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.processes.len(), 1);
    assert_eq!(config.processes["web"].cmd, "npm run dev");
}

#[test]
fn test_parse_full_config() {
    let yaml = r#"
processes:
  db:
    cmd: docker compose up postgres
    ready: "ready to accept connections"
  api:
    cmd: ./start-api-server
    cwd: ./backend
    env:
      DATABASE_URL: postgres://localhost:5432/mydb
    ready: "Listening on :8080"
    depends_on: [db]
"#;
    let config: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.processes["api"].cwd, Some("./backend".into()));
    assert_eq!(config.processes["api"].env.get("DATABASE_URL").unwrap(), "postgres://localhost:5432/mydb");
    assert_eq!(config.processes["api"].depends_on, vec!["db"]);
}

#[test]
fn test_topological_sort_concurrent_group() {
    let yaml = r#"
processes:
  db:
    cmd: start-db
  cache:
    cmd: start-cache
  api:
    cmd: start-api
    depends_on: [db, cache]
"#;
    let config: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
    let order = config.startup_order().unwrap();
    assert_eq!(order.len(), 2);
    assert!(order[0].contains(&"db".to_string()));
    assert!(order[0].contains(&"cache".to_string()));
    assert_eq!(order[0].len(), 2);
    assert_eq!(order[1], vec!["api"]);
}

#[test]
fn test_topological_sort_cycle_detection() {
    let yaml = "processes:\n  a:\n    cmd: x\n    depends_on: [b]\n  b:\n    cmd: y\n    depends_on: [a]\n";
    let config: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(config.startup_order().is_err());
}

#[test]
fn test_discover_config_walks_up() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("agent-procs.yaml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    writeln!(f, "processes:\n  web:\n    cmd: npm run dev").unwrap();
    let subdir = tmp.path().join("src/deep");
    std::fs::create_dir_all(&subdir).unwrap();
    assert_eq!(discover_config(&subdir), Some(config_path));
}

#[test]
fn test_discover_config_returns_none() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(discover_config(tmp.path()), None);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test test_config`
Expected: Compilation fails

- [ ] **Step 3: Implement config module**

Create `src/config.rs`:

```rust
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct ProjectConfig {
    pub processes: HashMap<String, ProcessDef>,
}

#[derive(Debug, Deserialize)]
pub struct ProcessDef {
    pub cmd: String,
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub ready: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

impl ProjectConfig {
    /// Returns groups of process names in startup order.
    /// Each group can be started concurrently; groups are sequential.
    pub fn startup_order(&self) -> Result<Vec<Vec<String>>, String> {
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

        for name in self.processes.keys() {
            in_degree.entry(name.as_str()).or_insert(0);
        }
        for (name, def) in &self.processes {
            for dep in &def.depends_on {
                if !self.processes.contains_key(dep) {
                    return Err(format!("unknown dependency: {} depends on {}", name, dep));
                }
                dependents.entry(dep.as_str()).or_default().push(name.as_str());
                *in_degree.entry(name.as_str()).or_insert(0) += 1;
            }
        }

        let mut groups = Vec::new();
        let mut remaining = in_degree.clone();

        loop {
            let mut ready: Vec<String> = remaining.iter()
                .filter(|(_, &deg)| deg == 0)
                .map(|(&name, _)| name.to_string())
                .collect();

            if ready.is_empty() {
                if remaining.is_empty() { break; }
                else { return Err("dependency cycle detected".into()); }
            }

            for name in &ready {
                remaining.remove(name.as_str());
                if let Some(deps) = dependents.get(name.as_str()) {
                    for dep in deps {
                        if let Some(deg) = remaining.get_mut(dep) { *deg -= 1; }
                    }
                }
            }
            ready.sort();
            groups.push(ready);
        }
        Ok(groups)
    }
}

pub fn discover_config(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join("agent-procs.yaml");
        if candidate.exists() { return Some(candidate); }
        if !dir.pop() { return None; }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test test_config`
Expected: All 6 tests pass

- [ ] **Step 5: Commit**

```bash
git add src/config.rs tests/test_config.rs src/lib.rs
git commit -m "feat: add config file parsing with topological sort"
```

---

### Task 5: Session management types

**Files:**
- Create: `src/session.rs`
- Create: `tests/test_session.rs`

- [ ] **Step 1: Write failing tests**

Create `tests/test_session.rs`:

```rust
use agent_procs::session::*;
use std::io::Write;
use tempfile::TempDir;

#[test]
fn test_is_daemon_alive_returns_false_for_missing_pid_file() {
    let tmp = TempDir::new().unwrap();
    assert!(!is_daemon_alive(&tmp.path().join("daemon.pid")));
}

#[test]
fn test_is_daemon_alive_returns_false_for_dead_pid() {
    let tmp = TempDir::new().unwrap();
    let pid_path = tmp.path().join("daemon.pid");
    let mut f = std::fs::File::create(&pid_path).unwrap();
    writeln!(f, "99999999").unwrap();
    assert!(!is_daemon_alive(&pid_path));
}

#[test]
fn test_is_daemon_alive_returns_true_for_current_process() {
    let tmp = TempDir::new().unwrap();
    let pid_path = tmp.path().join("daemon.pid");
    let mut f = std::fs::File::create(&pid_path).unwrap();
    writeln!(f, "{}", std::process::id()).unwrap();
    assert!(is_daemon_alive(&pid_path));
}

#[test]
fn test_next_process_id_increments() {
    let mut counter = IdCounter::new();
    assert_eq!(counter.next(), "p1");
    assert_eq!(counter.next(), "p2");
    assert_eq!(counter.next(), "p3");
}
```

- [ ] **Step 2: Implement session module**

Create `src/session.rs`:

```rust
use std::fs;
use std::path::Path;

pub fn is_daemon_alive(pid_path: &Path) -> bool {
    let content = match fs::read_to_string(pid_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let pid: i32 = match content.trim().parse() {
        Ok(p) => p,
        Err(_) => return false,
    };
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

pub struct IdCounter { next_id: u32 }

impl IdCounter {
    pub fn new() -> Self { Self { next_id: 1 } }
    pub fn next(&mut self) -> String {
        let id = format!("p{}", self.next_id);
        self.next_id += 1;
        id
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --test test_session`
Expected: All 4 tests pass

- [ ] **Step 4: Run all tests**

Run: `cargo test -- --test-threads=1`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/session.rs tests/test_session.rs src/lib.rs
git commit -m "feat: add session stale detection and ID counter"
```

---

## Chunk 2: Daemon Core

### Task 6: Daemon spawn (double-fork) and module stubs

**Files:**
- Create: `src/daemon/mod.rs`
- Create: `src/daemon/spawn.rs`
- Create: `src/daemon/server.rs` (stub)
- Create: `src/daemon/process_manager.rs` (stub)
- Create: `src/daemon/log_writer.rs` (stub)
- Create: `src/daemon/wait_engine.rs` (stub)

- [ ] **Step 1: Create daemon module structure**

Create `src/daemon/mod.rs`:

```rust
pub mod spawn;
pub mod server;
pub mod process_manager;
pub mod log_writer;
pub mod wait_engine;
```

- [ ] **Step 2: Implement daemon spawn**

Create `src/daemon/spawn.rs`:

```rust
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
```

- [ ] **Step 3: Create stubs for remaining daemon modules**

Create `src/daemon/server.rs`:

```rust
use std::path::Path;
use tokio::net::UnixListener;

pub async fn run(_session: &str, socket_path: &Path) {
    let listener = UnixListener::bind(socket_path).expect("failed to bind socket");
    loop {
        match listener.accept().await {
            Ok((_stream, _addr)) => {} // Implemented in Task 7
            Err(_) => break,
        }
    }
}
```

Create `src/daemon/process_manager.rs`:

```rust
// Implemented in Task 8
```

Create `src/daemon/log_writer.rs`:

```rust
// Implemented in Task 8 (output capture and broadcast)
```

Create `src/daemon/wait_engine.rs`:

```rust
// Implemented in Task 10
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build`
Expected: Compiles

- [ ] **Step 5: Commit**

```bash
git add src/daemon/ src/lib.rs
git commit -m "feat: add daemon double-fork spawn and module stubs"
```

---

### Task 7: Log writer and output broadcast

**Files:**
- Modify: `src/daemon/log_writer.rs`

The log_writer is responsible for capturing child process output, writing it to log files, and broadcasting lines to subscribers (for the wait engine). This module must exist before process_manager, since process_manager will call into it.

- [ ] **Step 1: Implement log_writer**

Replace `src/daemon/log_writer.rs`:

```rust
use crate::protocol::Stream as ProtoStream;
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;

/// A line of output from a child process.
#[derive(Debug, Clone)]
pub struct OutputLine {
    pub process: String,
    pub stream: ProtoStream,
    pub line: String,
}

/// Reads lines from a child's stdout or stderr, writes each line to a log file,
/// and broadcasts it to subscribers (wait engine, --follow clients).
pub async fn capture_output<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    log_path: &Path,
    process_name: &str,
    stream: ProtoStream,
    tx: broadcast::Sender<OutputLine>,
    max_bytes: u64,
) {
    let mut lines = BufReader::new(reader).lines();
    let mut file = match tokio::fs::File::create(log_path).await {
        Ok(f) => f,
        Err(_) => return,
    };
    let mut bytes_written: u64 = 0;

    while let Ok(Some(line)) = lines.next_line().await {
        // Write to log file (with rotation check)
        let line_bytes = line.len() as u64 + 1; // +1 for newline
        if max_bytes > 0 && bytes_written + line_bytes > max_bytes {
            // Rotate: close current, rename to .1, start fresh
            drop(file);
            let rotated = log_path.with_extension(
                format!("{}.1", log_path.extension().unwrap_or_default().to_string_lossy())
            );
            let _ = tokio::fs::rename(log_path, &rotated).await;
            file = match tokio::fs::File::create(log_path).await {
                Ok(f) => f,
                Err(_) => return,
            };
            bytes_written = 0;
        }

        let _ = file.write_all(line.as_bytes()).await;
        let _ = file.write_all(b"\n").await;
        let _ = file.flush().await;
        bytes_written += line_bytes;

        // Broadcast to wait engine / followers
        let _ = tx.send(OutputLine {
            process: process_name.to_string(),
            stream: stream.clone(),
            line,
        });
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add src/daemon/log_writer.rs
git commit -m "feat: add log writer with output capture, broadcast, and rotation"
```

---

### Task 8: Process manager

**Files:**
- Modify: `src/daemon/process_manager.rs`

Uses `Stdio::piped()` so the daemon reads child output (enabling pattern matching). Delegates output capture to `log_writer::capture_output`.

- [ ] **Step 1: Implement process manager**

Replace `src/daemon/process_manager.rs`:

```rust
use crate::daemon::log_writer::{self, OutputLine};
use crate::paths;
use crate::protocol::{ProcessInfo, ProcessState, Response, Stream as ProtoStream};
use crate::session::IdCounter;
use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use tokio::sync::broadcast;

const DEFAULT_MAX_LOG_BYTES: u64 = 50 * 1024 * 1024; // 50MB

pub struct ManagedProcess {
    pub name: String,
    pub id: String,
    pub command: String,
    pub child: Option<Child>,
    pub pid: u32,
    pub started_at: Instant,
    pub exit_code: Option<i32>,
}

pub struct ProcessManager {
    processes: HashMap<String, ManagedProcess>,
    id_counter: IdCounter,
    session: String,
    pub output_tx: broadcast::Sender<OutputLine>,
}

impl ProcessManager {
    pub fn new(session: &str) -> Self {
        let (output_tx, _) = broadcast::channel(1024);
        Self {
            processes: HashMap::new(),
            id_counter: IdCounter::new(),
            session: session.to_string(),
            output_tx,
        }
    }

    pub async fn spawn_process(&mut self, command: &str, name: Option<String>, cwd: Option<&str>) -> Response {
        let id = self.id_counter.next();
        let name = name.unwrap_or_else(|| id.clone());

        if self.processes.contains_key(&name) {
            return Response::Error { code: 1, message: format!("process already exists: {}", name) };
        }

        let log_dir = paths::log_dir(&self.session);
        let _ = std::fs::create_dir_all(&log_dir);

        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let mut child = match cmd.spawn()
        {
            Ok(c) => c,
            Err(e) => return Response::Error { code: 1, message: format!("failed to spawn: {}", e) },
        };

        let pid = child.id().unwrap_or(0);

        // Spawn output capture tasks via log_writer
        if let Some(stdout) = child.stdout.take() {
            let tx = self.output_tx.clone();
            let pname = name.clone();
            let path = log_dir.join(format!("{}.stdout", name));
            tokio::spawn(async move {
                log_writer::capture_output(stdout, &path, &pname, ProtoStream::Stdout, tx, DEFAULT_MAX_LOG_BYTES).await;
            });
        }
        if let Some(stderr) = child.stderr.take() {
            let tx = self.output_tx.clone();
            let pname = name.clone();
            let path = log_dir.join(format!("{}.stderr", name));
            tokio::spawn(async move {
                log_writer::capture_output(stderr, &path, &pname, ProtoStream::Stderr, tx, DEFAULT_MAX_LOG_BYTES).await;
            });
        }

        self.processes.insert(name.clone(), ManagedProcess {
            name: name.clone(), id: id.clone(), command: command.to_string(),
            child: Some(child), pid, started_at: Instant::now(), exit_code: None,
        });

        Response::RunOk { name, id, pid }
    }

    pub async fn stop_process(&mut self, target: &str) -> Response {
        let proc = match self.find_mut(target) {
            Some(p) => p,
            None => return Response::Error { code: 2, message: format!("process not found: {}", target) },
        };

        if let Some(ref child) = proc.child {
            let raw_pid = child.id().unwrap_or(0) as i32;
            if raw_pid > 0 {
                // Send SIGTERM first
                let pid = nix::unistd::Pid::from_raw(raw_pid);
                let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
            }
        }

        // Wait up to 10s for graceful exit, then SIGKILL
        if let Some(ref mut child) = proc.child {
            let wait_result = tokio::time::timeout(
                Duration::from_secs(10),
                child.wait()
            ).await;

            match wait_result {
                Ok(Ok(status)) => {
                    proc.exit_code = status.code();
                }
                _ => {
                    // Timed out or error — force kill
                    let _ = child.kill().await;
                    let status = child.wait().await;
                    proc.exit_code = status.ok().and_then(|s| s.code());
                }
            }
            proc.child = None;
        }

        Response::Ok { message: format!("stopped {}", target) }
    }

    pub async fn stop_all(&mut self) -> Response {
        let names: Vec<String> = self.processes.keys().cloned().collect();
        for name in names {
            self.stop_process(&name).await;
        }
        Response::Ok { message: "all processes stopped".into() }
    }

    pub async fn restart_process(&mut self, target: &str) -> Response {
        let (command, name) = match self.find(target) {
            Some(p) => (p.command.clone(), p.name.clone()),
            None => return Response::Error { code: 2, message: format!("process not found: {}", target) },
        };
        self.stop_process(target).await;
        self.processes.remove(&name);
        self.spawn_process(&command, Some(name), None).await
    }

    pub fn status(&mut self) -> Response {
        self.refresh_exit_states();
        let mut infos: Vec<ProcessInfo> = self.processes.values()
            .map(|p| ProcessInfo {
                name: p.name.clone(), id: p.id.clone(), pid: p.pid,
                state: if p.child.is_some() { ProcessState::Running } else { ProcessState::Exited },
                exit_code: p.exit_code,
                uptime_secs: if p.child.is_some() { Some(p.started_at.elapsed().as_secs()) } else { None },
                command: p.command.clone(),
            })
            .collect();
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        Response::Status { processes: infos }
    }

    pub fn is_process_exited(&mut self, target: &str) -> Option<Option<i32>> {
        self.refresh_exit_states();
        self.find(target).map(|p| if p.child.is_none() { p.exit_code } else { None })
    }

    fn refresh_exit_states(&mut self) {
        for proc in self.processes.values_mut() {
            if proc.child.is_some() && proc.exit_code.is_none() {
                if let Some(ref mut child) = proc.child {
                    if let Ok(Some(status)) = child.try_wait() {
                        proc.exit_code = status.code();
                        proc.child = None;
                    }
                }
            }
        }
    }

    pub fn has_process(&self, target: &str) -> bool {
        self.find(target).is_some()
    }

    fn find(&self, target: &str) -> Option<&ManagedProcess> {
        self.processes.get(target)
            .or_else(|| self.processes.values().find(|p| p.id == target))
    }

    fn find_mut(&mut self, target: &str) -> Option<&mut ManagedProcess> {
        if self.processes.contains_key(target) {
            self.processes.get_mut(target)
        } else {
            self.processes.values_mut().find(|p| p.id == target)
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add src/daemon/process_manager.rs
git commit -m "feat: add process manager with piped output, SIGTERM escalation"
```

---

### Task 9: Wait engine

**Files:**
- Modify: `src/daemon/wait_engine.rs`

The wait engine subscribes to the output broadcast channel and checks incoming lines against a pattern. It resolves when the pattern matches, the process exits, or a timeout is reached.

- [ ] **Step 1: Implement wait engine**

Replace `src/daemon/wait_engine.rs`:

```rust
use crate::daemon::log_writer::OutputLine;
use crate::protocol::Response;
use regex;
use std::time::Duration;
use tokio::sync::broadcast;

/// Wait for a condition on a process's output.
/// Returns a Response indicating match, exit, or timeout.
pub async fn wait_for(
    mut output_rx: broadcast::Receiver<OutputLine>,
    target: &str,
    pattern: Option<&str>,
    use_regex: bool,
    wait_exit: bool,
    timeout: Duration,
    // Closure to check if process has already exited
    mut check_exit: impl FnMut() -> Option<Option<i32>>,
) -> Response {
    let compiled_regex = if use_regex {
        pattern.and_then(|p| regex::Regex::new(p).ok())
    } else {
        None
    };

    // Check if already exited before we start waiting
    if wait_exit {
        if let Some(Some(code)) = check_exit() {
            return Response::WaitExited { exit_code: code };
        }
    }

    tokio::select! {
        result = async {
            loop {
                match output_rx.recv().await {
                    Ok(line) => {
                        if line.process != target { continue; }
                        if let Some(pat) = pattern {
                            let matched = if let Some(ref re) = compiled_regex {
                                re.is_match(&line.line)
                            } else {
                                line.line.contains(pat)
                            };
                            if matched {
                                return Response::WaitMatch { line: line.line };
                            }
                        }
                        // After each line, check if process exited (for --exit mode)
                        if wait_exit {
                            if let Some(Some(code)) = check_exit() {
                                return Response::WaitExited { exit_code: code };
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => {
                        if wait_exit {
                            // Channel closed — process likely exited
                            if let Some(Some(code)) = check_exit() {
                                return Response::WaitExited { exit_code: code };
                            }
                        }
                        return Response::Error { code: 1, message: "output channel closed".into() };
                    }
                }
            }
        } => result,
        _ = tokio::time::sleep(timeout) => Response::WaitTimeout,
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add src/daemon/wait_engine.rs
git commit -m "feat: add wait engine with pattern matching and timeout"
```

---

### Task 10: Daemon socket server with request routing

**Files:**
- Modify: `src/daemon/server.rs`

- [ ] **Step 1: Implement the socket server**

Replace `src/daemon/server.rs`:

```rust
use crate::daemon::wait_engine;
use crate::protocol::{Request, Response};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::Mutex;

use super::process_manager::ProcessManager;

pub struct DaemonState {
    pub process_manager: ProcessManager,
    pub session: String,
    pub should_shutdown: bool,
}

pub async fn run(session: &str, socket_path: &Path) {
    let state = Arc::new(Mutex::new(DaemonState {
        process_manager: ProcessManager::new(session),
        session: session.to_string(),
        should_shutdown: false,
    }));

    let listener = UnixListener::bind(socket_path).expect("failed to bind socket");

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(_) => break,
        };

        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let (reader, writer) = stream.into_split();
            let writer = Arc::new(Mutex::new(writer));
            let mut lines = BufReader::new(reader).lines();

            while let Ok(Some(line)) = lines.next_line().await {
                let request: Request = match serde_json::from_str(&line) {
                    Ok(r) => r,
                    Err(e) => {
                        let resp = Response::Error { code: 1, message: format!("invalid request: {}", e) };
                        let _ = send_response(&writer, &resp).await;
                        continue;
                    }
                };

                let is_shutdown = matches!(request, Request::Shutdown);

                let response = handle_request(&state, request).await;
                let _ = send_response(&writer, &response).await;

                if is_shutdown {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    std::process::exit(0);
                }
            }
        });
    }
}

async fn handle_request(state: &Arc<Mutex<DaemonState>>, request: Request) -> Response {
    match request {
        Request::Run { command, name, cwd } => {
            state.lock().await.process_manager.spawn_process(&command, name, cwd.as_deref()).await
        }
        Request::Stop { target } => {
            state.lock().await.process_manager.stop_process(&target).await
        }
        Request::StopAll => {
            state.lock().await.process_manager.stop_all().await
        }
        Request::Restart { target } => {
            state.lock().await.process_manager.restart_process(&target).await
        }
        Request::Status => {
            state.lock().await.process_manager.status()
        }
        Request::Wait { target, until, regex, exit, timeout_secs } => {
            // Check process exists
            {
                let s = state.lock().await;
                if !s.process_manager.has_process(&target) {
                    return Response::Error { code: 2, message: format!("process not found: {}", target) };
                }
            }
            // Subscribe to output and delegate to wait engine
            let output_rx = state.lock().await.process_manager.output_tx.subscribe();
            let timeout = Duration::from_secs(timeout_secs.unwrap_or(30));
            let state_clone = Arc::clone(state);
            let target_clone = target.clone();
            wait_engine::wait_for(
                output_rx,
                &target,
                until.as_deref(),
                regex,
                exit,
                timeout,
                move || {
                    // This is called synchronously from the wait loop
                    // We can't hold the lock across the whole wait, so we check briefly
                    let state = state_clone.clone();
                    let target = target_clone.clone();
                    // Use try_lock to avoid deadlock
                    match state.try_lock() {
                        Ok(mut s) => s.process_manager.is_process_exited(&target),
                        Err(_) => None,
                    }
                },
            ).await
        }
        Request::Logs { .. } => {
            // Logs are read directly from files by the CLI — no daemon involvement needed
            Response::Error { code: 1, message: "logs are read directly from disk by CLI".into() }
        }
        Request::Shutdown => {
            state.lock().await.process_manager.stop_all().await;
            Response::Ok { message: "daemon shutting down".into() }
        }
    }
}

async fn send_response(writer: &Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>, response: &Response) -> std::io::Result<()> {
    let mut w = writer.lock().await;
    let mut json = serde_json::to_string(response).unwrap();
    json.push('\n');
    w.write_all(json.as_bytes()).await?;
    w.flush().await
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add src/daemon/server.rs
git commit -m "feat: add daemon socket server with request routing"
```

---

### Task 11: CLI client helpers and wiring

**Files:**
- Create: `src/cli/mod.rs`
- Create: `src/cli/run.rs`
- Create: `src/cli/stop.rs`
- Create: `src/cli/restart.rs`
- Create: `src/cli/status.rs`
- Create: `src/cli/logs.rs` (stub)
- Create: `src/cli/wait.rs` (stub)
- Create: `src/cli/up.rs` (stub)
- Create: `src/cli/down.rs` (stub)
- Create: `src/cli/session_cmd.rs` (stub)
- Modify: `src/main.rs` (wire up commands)

- [ ] **Step 1: Create CLI client module with shared helpers**

Create `src/cli/mod.rs`:

```rust
pub mod run;
pub mod stop;
pub mod restart;
pub mod status;
pub mod logs;
pub mod wait;
pub mod up;
pub mod down;
pub mod session_cmd;

use crate::protocol::{Request, Response};
use crate::paths;
use crate::session;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

pub async fn connect(session: &str, auto_spawn: bool) -> Result<UnixStream, String> {
    let socket = paths::socket_path(session);
    let pid = paths::pid_path(session);

    if !session::is_daemon_alive(&pid) {
        if auto_spawn {
            crate::daemon::spawn::spawn_daemon(session)
                .map_err(|e| format!("failed to spawn daemon: {}", e))?;
        } else {
            return Err("no daemon running for this session".into());
        }
    }

    UnixStream::connect(&socket)
        .await
        .map_err(|e| format!("failed to connect to daemon: {}", e))
}

pub async fn request(session: &str, req: &Request, auto_spawn: bool) -> Result<Response, String> {
    let stream = connect(session, auto_spawn).await?;
    let (reader, mut writer) = stream.into_split();

    let mut json = serde_json::to_string(req).unwrap();
    json.push('\n');
    writer.write_all(json.as_bytes()).await.map_err(|e| format!("write error: {}", e))?;
    writer.flush().await.map_err(|e| format!("flush error: {}", e))?;

    let mut lines = BufReader::new(reader);
    let mut line = String::new();
    lines.read_line(&mut line).await.map_err(|e| format!("read error: {}", e))?;

    serde_json::from_str(&line).map_err(|e| format!("parse error: {}", e))
}
```

- [ ] **Step 2: Create CLI command implementations**

Create `src/cli/run.rs`:

```rust
use crate::protocol::{Request, Response};

pub async fn execute(session: &str, command: &str, name: Option<String>) -> i32 {
    let req = Request::Run { command: command.into(), name, cwd: None };
    match crate::cli::request(session, &req, true).await {
        Ok(Response::RunOk { name, id, pid }) => { println!("{} (id: {}, pid: {})", name, id, pid); 0 }
        Ok(Response::Error { code, message }) => { eprintln!("error: {}", message); code }
        Ok(_) => { eprintln!("unexpected response"); 1 }
        Err(e) => { eprintln!("error: {}", e); 1 }
    }
}
```

Create `src/cli/stop.rs`:

```rust
use crate::protocol::{Request, Response};

pub async fn execute(session: &str, target: &str) -> i32 {
    let req = Request::Stop { target: target.into() };
    match crate::cli::request(session, &req, false).await {
        Ok(Response::Ok { message }) => { println!("{}", message); 0 }
        Ok(Response::Error { code, message }) => { eprintln!("error: {}", message); code }
        _ => 1,
    }
}

pub async fn execute_all(session: &str) -> i32 {
    let req = Request::StopAll;
    match crate::cli::request(session, &req, false).await {
        Ok(Response::Ok { message }) => { println!("{}", message); 0 }
        Ok(Response::Error { code, message }) => { eprintln!("error: {}", message); code }
        _ => 1,
    }
}
```

Create `src/cli/restart.rs`:

```rust
use crate::protocol::{Request, Response};

pub async fn execute(session: &str, target: &str) -> i32 {
    let req = Request::Restart { target: target.into() };
    match crate::cli::request(session, &req, false).await {
        Ok(Response::RunOk { name, id, pid }) => { println!("restarted {} (id: {}, pid: {})", name, id, pid); 0 }
        Ok(Response::Error { code, message }) => { eprintln!("error: {}", message); code }
        _ => 1,
    }
}
```

Create `src/cli/status.rs`:

```rust
use crate::protocol::{Request, Response, ProcessState};

pub async fn execute(session: &str, json: bool) -> i32 {
    let req = Request::Status;
    match crate::cli::request(session, &req, false).await {
        Ok(Response::Status { processes }) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&processes).unwrap());
            } else {
                println!("{:<12} {:<8} {:<10} {:<6} {}", "NAME", "PID", "STATE", "EXIT", "UPTIME");
                for p in &processes {
                    let state = match p.state { ProcessState::Running => "running", ProcessState::Exited => "exited" };
                    let exit = p.exit_code.map(|c| c.to_string()).unwrap_or("-".into());
                    let uptime = p.uptime_secs.map(format_uptime).unwrap_or("-".into());
                    println!("{:<12} {:<8} {:<10} {:<6} {}", p.name, p.pid, state, exit, uptime);
                }
            }
            0
        }
        Ok(Response::Error { code, message }) => { eprintln!("error: {}", message); code }
        _ => 1,
    }
}

fn format_uptime(secs: u64) -> String {
    if secs < 60 { format!("{}s", secs) }
    else if secs < 3600 { format!("{}m{}s", secs / 60, secs % 60) }
    else { format!("{}h{}m", secs / 3600, (secs % 3600) / 60) }
}
```

Create stubs for `logs.rs`, `wait.rs`, `up.rs`, `down.rs`, `session_cmd.rs` — each with a placeholder:

```rust
pub async fn execute(/* ... */) -> i32 {
    eprintln!("not yet implemented");
    1
}
```

- [ ] **Step 3: Wire up commands in main.rs**

Update `src/main.rs` to dispatch to CLI modules. Replace the placeholder `main` body with:

```rust
let exit_code = match cli.command {
    Commands::Run { command, name } => agent_procs::cli::run::execute(session, &command, name).await,
    Commands::Stop { target } => agent_procs::cli::stop::execute(session, &target).await,
    Commands::StopAll => agent_procs::cli::stop::execute_all(session).await,
    Commands::Restart { target } => agent_procs::cli::restart::execute(session, &target).await,
    Commands::Status { json } => agent_procs::cli::status::execute(session, json).await,
    Commands::Logs { target, tail, follow, stderr, all, timeout } => {
        agent_procs::cli::logs::execute(session, target.as_deref(), tail, follow, stderr, all, timeout).await
    }
    Commands::Wait { target, until, regex, exit, timeout } => {
        agent_procs::cli::wait::execute(session, &target, until, regex, exit, timeout).await
    }
    Commands::Up { only, config } => {
        agent_procs::cli::up::execute(session, only.as_deref(), config.as_deref()).await
    }
    Commands::Down => agent_procs::cli::down::execute(session).await,
    Commands::Session { command } => match command {
        SessionCommands::List => agent_procs::cli::session_cmd::list().await,
        SessionCommands::Clean => agent_procs::cli::session_cmd::clean().await,
    },
};
std::process::exit(exit_code);
```

- [ ] **Step 4: Create test helpers**

Create `tests/helpers/mod.rs`:

```rust
use std::path::PathBuf;
use tempfile::TempDir;

pub struct TestContext {
    pub runtime_dir: TempDir,
    pub state_dir: TempDir,
    pub session: String,
}

impl TestContext {
    pub fn new(session: &str) -> Self {
        Self {
            runtime_dir: TempDir::new().unwrap(),
            state_dir: TempDir::new().unwrap(),
            session: session.into(),
        }
    }

    pub fn set_env(&self) {
        std::env::set_var("XDG_RUNTIME_DIR", self.runtime_dir.path());
        std::env::set_var("XDG_STATE_HOME", self.state_dir.path());
    }
}
```

- [ ] **Step 5: Write integration test for run/status/stop**

Create `tests/test_run_stop.rs`:

```rust
mod helpers;
use assert_cmd::Command;
use helpers::TestContext;

// All integration tests MUST run with --test-threads=1 (env var mutation).

#[test]
fn test_run_and_status() {
    let ctx = TestContext::new("test-run-status");
    ctx.set_env();

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run", "sleep 60", "--name", "sleeper"])
        .output().unwrap();
    assert!(output.status.success(), "run failed: {}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("sleeper"));

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "status"])
        .output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("sleeper"));
    assert!(stdout.contains("running"));

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop", "sleeper"])
        .output().unwrap();
    assert!(output.status.success());

    // Cleanup daemon
    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_stop_nonexistent_returns_error() {
    let ctx = TestContext::new("test-stop-missing");
    ctx.set_env();

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run", "sleep 60", "--name", "bg"])
        .output().unwrap();

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop", "nonexistent"])
        .output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not found") || !output.status.success());

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}
```

- [ ] **Step 6: Run integration tests**

Run: `cargo test --test test_run_stop -- --test-threads=1`
Expected: Both tests pass

- [ ] **Step 7: Commit**

```bash
git add src/cli/ src/main.rs tests/helpers/ tests/test_run_stop.rs
git commit -m "feat: add CLI client, wire up run/stop/status, first integration tests"
```

---

## Chunk 3: Logs, Wait, and Config Startup

### Task 12: Logs command

**Files:**
- Modify: `src/cli/logs.rs`
- Create: `tests/test_logs.rs`

Logs are read directly from disk (the daemon writes them). No daemon protocol needed for basic `--tail`. Note: `--follow` is deferred to a future version — it would require streaming from the daemon's broadcast channel.

- [ ] **Step 1: Write integration test**

Create `tests/test_logs.rs`:

```rust
mod helpers;
use assert_cmd::Command;
use helpers::TestContext;
use std::thread;
use std::time::Duration;

#[test]
fn test_logs_tail() {
    let ctx = TestContext::new("test-logs-tail");
    ctx.set_env();

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run", "echo hello world", "--name", "echoer"])
        .output().unwrap();

    thread::sleep(Duration::from_millis(500));

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "logs", "echoer", "--tail", "10"])
        .output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("hello world"));

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}

#[test]
fn test_logs_all_interleaved() {
    let ctx = TestContext::new("test-logs-all");
    ctx.set_env();

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run", "echo from-alpha", "--name", "alpha"])
        .output().unwrap();
    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run", "echo from-beta", "--name", "beta"])
        .output().unwrap();

    thread::sleep(Duration::from_millis(500));

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "logs", "--all", "--tail", "10"])
        .output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[alpha]"));
    assert!(stdout.contains("[beta]"));

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}
```

- [ ] **Step 2: Implement logs command**

Replace `src/cli/logs.rs`:

```rust
use crate::paths;
use std::io::{BufRead, BufReader};
use std::fs::File;

pub async fn execute(
    session: &str, target: Option<&str>, tail: usize,
    _follow: bool, stderr: bool, all: bool, _timeout: Option<u64>,
) -> i32 {
    // Note: --follow is deferred. The flag is accepted but ignored with a warning.
    if _follow {
        eprintln!("warning: --follow is not yet implemented, showing --tail output instead");
    }

    let log_dir = paths::log_dir(session);

    if all || target.is_none() {
        return show_all_logs(&log_dir, tail);
    }

    let target = target.unwrap();
    let stream = if stderr { "stderr" } else { "stdout" };
    let path = log_dir.join(format!("{}.{}", target, stream));

    if !path.exists() {
        eprintln!("error: no logs for process '{}' ({})", target, stream);
        return 2;
    }

    match tail_file(&path, tail) {
        Ok(lines) => { for line in lines { println!("{}", line); } 0 }
        Err(e) => { eprintln!("error reading logs: {}", e); 1 }
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
        let proc_name = name.trim_end_matches(".stdout");
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
    let lines: Vec<String> = BufReader::new(file).lines().collect::<Result<_, _>>()?;
    let start = lines.len().saturating_sub(n);
    Ok(lines[start..].to_vec())
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --test test_logs -- --test-threads=1`
Expected: Both tests pass

- [ ] **Step 4: Commit**

```bash
git add src/cli/logs.rs tests/test_logs.rs
git commit -m "feat: add logs command with --tail, --stderr, --all"
```

---

### Task 13: Wait command (CLI)

**Files:**
- Modify: `src/cli/wait.rs`
- Create: `tests/test_wait.rs`

- [ ] **Step 1: Write integration tests**

Create `tests/test_wait.rs`:

```rust
mod helpers;
use assert_cmd::Command;
use helpers::TestContext;
use std::time::Duration;

#[test]
fn test_wait_until_pattern() {
    let ctx = TestContext::new("test-wait-pattern");
    ctx.set_env();

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run",
            "sleep 1 && echo 'Server ready on port 3000' && sleep 60",
            "--name", "server"])
        .output().unwrap();

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "wait", "server",
            "--until", "ready on port", "--timeout", "10"])
        .timeout(Duration::from_secs(15))
        .output().unwrap();
    assert!(output.status.success());

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}

#[test]
fn test_wait_exit() {
    let ctx = TestContext::new("test-wait-exit");
    ctx.set_env();

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run", "echo done && exit 0", "--name", "quick"])
        .output().unwrap();

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "wait", "quick", "--exit", "--timeout", "10"])
        .timeout(Duration::from_secs(15))
        .output().unwrap();
    assert!(output.status.success());

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}

#[test]
fn test_wait_timeout() {
    let ctx = TestContext::new("test-wait-timeout");
    ctx.set_env();

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run", "sleep 60", "--name", "forever"])
        .output().unwrap();

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "wait", "forever",
            "--until", "this will never appear", "--timeout", "2"])
        .timeout(Duration::from_secs(10))
        .output().unwrap();
    assert_eq!(output.status.code(), Some(1));

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}
```

- [ ] **Step 2: Implement wait CLI command**

Replace `src/cli/wait.rs`:

```rust
use crate::protocol::{Request, Response};

pub async fn execute(
    session: &str, target: &str, until: Option<String>,
    regex: bool, exit: bool, timeout: Option<u64>,
) -> i32 {
    let req = Request::Wait {
        target: target.into(), until, regex, exit,
        timeout_secs: timeout,
    };
    match crate::cli::request(session, &req, false).await {
        Ok(Response::WaitMatch { line }) => { println!("{}", line); 0 }
        Ok(Response::WaitExited { exit_code }) => { println!("exited with code {}", exit_code); 0 }
        Ok(Response::WaitTimeout) => { eprintln!("timeout"); 1 }
        Ok(Response::Error { code, message }) => { eprintln!("error: {}", message); code }
        Ok(_) => { eprintln!("unexpected response"); 1 }
        Err(e) => { eprintln!("error: {}", e); 1 }
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --test test_wait -- --test-threads=1`
Expected: All 3 tests pass

- [ ] **Step 4: Commit**

```bash
git add src/cli/wait.rs tests/test_wait.rs
git commit -m "feat: add wait command with pattern matching, exit, and timeout"
```

---

### Task 14: Config-based up/down commands

**Files:**
- Modify: `src/cli/up.rs`
- Modify: `src/cli/down.rs`
- Create: `tests/test_up_down.rs`

- [ ] **Step 1: Write integration test**

Create `tests/test_up_down.rs`:

```rust
mod helpers;
use assert_cmd::Command;
use helpers::TestContext;
use std::io::Write;
use std::time::Duration;

#[test]
fn test_up_starts_all_processes() {
    let ctx = TestContext::new("test-up-all");
    ctx.set_env();

    let config_dir = tempfile::TempDir::new().unwrap();
    let config_path = config_dir.path().join("agent-procs.yaml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    write!(f, "processes:\n  alpha:\n    cmd: \"echo alpha-ready && sleep 60\"\n    ready: \"alpha-ready\"\n  beta:\n    cmd: \"echo beta-ready && sleep 60\"\n    ready: \"beta-ready\"\n").unwrap();

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "up", "--config", config_path.to_str().unwrap()])
        .timeout(Duration::from_secs(30))
        .output().unwrap();
    assert!(output.status.success(), "up failed: {}", String::from_utf8_lossy(&output.stderr));

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "status"])
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("alpha"));
    assert!(stdout.contains("beta"));

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "down"]).output();
}

#[test]
fn test_up_respects_depends_on() {
    let ctx = TestContext::new("test-up-deps");
    ctx.set_env();

    let config_dir = tempfile::TempDir::new().unwrap();
    let config_path = config_dir.path().join("agent-procs.yaml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    write!(f, "processes:\n  db:\n    cmd: \"echo db-ready && sleep 60\"\n    ready: \"db-ready\"\n  api:\n    cmd: \"echo api-ready && sleep 60\"\n    ready: \"api-ready\"\n    depends_on: [db]\n").unwrap();

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "up", "--config", config_path.to_str().unwrap()])
        .timeout(Duration::from_secs(30))
        .output().unwrap();
    assert!(output.status.success());

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "status"])
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("db"));
    assert!(stdout.contains("api"));

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "down"]).output();
}
```

- [ ] **Step 2: Implement up command**

Replace `src/cli/up.rs`:

```rust
use crate::cli;
use crate::config::{discover_config, ProjectConfig};
use crate::protocol::{Request, Response};

pub async fn execute(session: &str, only: Option<&str>, config_path: Option<&str>) -> i32 {
    let path = match config_path {
        Some(p) => std::path::PathBuf::from(p),
        None => match discover_config(&std::env::current_dir().unwrap()) {
            Some(p) => p,
            None => { eprintln!("error: no agent-procs.yaml found"); return 1; }
        },
    };

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => { eprintln!("error: cannot read config: {}", e); return 1; }
    };

    let config: ProjectConfig = match serde_yaml::from_str(&content) {
        Ok(c) => c,
        Err(e) => { eprintln!("error: invalid config: {}", e); return 1; }
    };

    let only_set: Option<Vec<&str>> = only.map(|s| s.split(',').collect());

    let groups = match config.startup_order() {
        Ok(g) => g,
        Err(e) => { eprintln!("error: {}", e); return 1; }
    };

    for group in &groups {
        for name in group {
            if let Some(ref only) = only_set {
                if !only.contains(&name.as_str()) { continue; }
            }

            let def = &config.processes[name];
            let mut cmd = def.cmd.clone();
            if !def.env.is_empty() {
                let env_prefix: Vec<String> = def.env.iter()
                    .map(|(k, v)| format!("{}={}", k, shell_escape(v)))
                    .collect();
                cmd = format!("{} {}", env_prefix.join(" "), cmd);
            }

            // Resolve cwd relative to config file directory
            let resolved_cwd = def.cwd.as_ref().map(|c| {
                let p = std::path::Path::new(c);
                if p.is_relative() {
                    path.parent().unwrap_or(std::path::Path::new(".")).join(p).to_string_lossy().to_string()
                } else {
                    c.clone()
                }
            });

            // Start the process
            let req = Request::Run { command: cmd, name: Some(name.clone()), cwd: resolved_cwd };
            match cli::request(session, &req, true).await {
                Ok(Response::RunOk { name, id, pid }) => {
                    println!("started {} (id: {}, pid: {})", name, id, pid);
                }
                Ok(Response::Error { code, message }) => {
                    eprintln!("error starting {}: {}", name, message);
                    return code;
                }
                _ => return 1,
            }

            // Wait for ready pattern
            if let Some(ref ready) = def.ready {
                let req = Request::Wait {
                    target: name.clone(), until: Some(ready.clone()),
                    regex: false, exit: false, timeout_secs: Some(30),
                };
                match cli::request(session, &req, false).await {
                    Ok(Response::WaitMatch { .. }) => println!("{} is ready", name),
                    Ok(Response::WaitTimeout) => {
                        eprintln!("warning: {} did not become ready within 30s", name);
                    }
                    Ok(Response::Error { message, .. }) => {
                        eprintln!("error waiting for {}: {}", name, message);
                        return 1;
                    }
                    _ => {}
                }
            }
        }
    }

    println!("all processes started");
    0
}

fn shell_escape(s: &str) -> String {
    if s.contains(' ') || s.contains('\'') || s.contains('"') {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}
```

Replace `src/cli/down.rs`:

```rust
pub async fn execute(session: &str) -> i32 {
    crate::cli::stop::execute_all(session).await
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --test test_up_down -- --test-threads=1`
Expected: Both tests pass

- [ ] **Step 4: Commit**

```bash
git add src/cli/up.rs src/cli/down.rs tests/test_up_down.rs
git commit -m "feat: add up/down commands for config-based startup"
```

---

## Chunk 4: Session Management, E2E Test, Skill, and Cleanup

### Task 15: Session list and clean commands

**Files:**
- Modify: `src/cli/session_cmd.rs`
- Create: `tests/test_session_mgmt.rs`

- [ ] **Step 1: Write test**

Create `tests/test_session_mgmt.rs`:

```rust
mod helpers;
use assert_cmd::Command;
use helpers::TestContext;

#[test]
fn test_session_list_shows_active() {
    let ctx = TestContext::new("test-sess-list");
    ctx.set_env();

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run", "sleep 60", "--name", "bg"])
        .output().unwrap();

    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["session", "list"])
        .output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains(&ctx.session));

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}

#[test]
fn test_session_clean_removes_stale() {
    let ctx = TestContext::new("test-sess-clean");
    ctx.set_env();

    // Start and then kill daemon to create stale session
    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run", "sleep 60", "--name", "bg"])
        .output().unwrap();
    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();

    // Write a fake PID to make it look stale
    let pid_path = ctx.runtime_dir.path()
        .join(format!("agent-procs/sessions/{}/daemon.pid", ctx.session));
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

- [ ] **Step 2: Implement session commands**

Replace `src/cli/session_cmd.rs`:

```rust
use crate::paths;
use crate::session;

pub async fn list() -> i32 {
    let runtime_base = paths::sessions_base_dir();

    if !runtime_base.exists() {
        println!("no active sessions");
        return 0;
    }

    let entries = match std::fs::read_dir(&runtime_base) {
        Ok(e) => e,
        Err(_) => { println!("no active sessions"); return 0; }
    };

    println!("{:<20} {}", "SESSION", "STATUS");
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let pid_path = entry.path().join("daemon.pid");
        let status = if session::is_daemon_alive(&pid_path) { "running" } else { "stale" };
        println!("{:<20} {}", name, status);
    }
    0
}

pub async fn clean() -> i32 {
    let runtime_base = paths::sessions_base_dir();

    let entries = match std::fs::read_dir(&runtime_base) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let pid_path = entry.path().join("daemon.pid");
        if !session::is_daemon_alive(&pid_path) {
            let name = entry.file_name().to_string_lossy().to_string();
            let _ = std::fs::remove_dir_all(entry.path());
            let _ = std::fs::remove_dir_all(paths::state_dir(&name));
            println!("cleaned stale session: {}", name);
        }
    }
    0
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --test test_session_mgmt -- --test-threads=1`
Expected: Test passes

- [ ] **Step 4: Commit**

```bash
git add src/cli/session_cmd.rs tests/test_session_mgmt.rs
git commit -m "feat: add session list and clean commands"
```

---

### Task 16: End-to-end agent workflow test

**Files:**
- Create: `tests/test_e2e.rs`

- [ ] **Step 1: Write e2e test**

Create `tests/test_e2e.rs`:

```rust
mod helpers;
use assert_cmd::Command;
use helpers::TestContext;
use std::time::Duration;

#[test]
fn test_agent_workflow() {
    let ctx = TestContext::new("test-e2e");
    ctx.set_env();

    // 1. Start a "server"
    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run",
            "echo 'Starting...' && sleep 0.5 && echo 'Server ready on port 3000' && sleep 60",
            "--name", "server"])
        .output().unwrap();
    assert!(output.status.success());

    // 2. Wait for ready
    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "wait", "server",
            "--until", "ready on port", "--timeout", "10"])
        .timeout(Duration::from_secs(15))
        .output().unwrap();
    assert!(output.status.success());

    // 3. Check status (JSON)
    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "status", "--json"])
        .output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"state\":\"running\""));

    // 4. Read logs
    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "logs", "server", "--tail", "5"])
        .output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Server ready"));

    // 5. Stop
    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"])
        .output().unwrap();
    assert!(output.status.success());
}
```

- [ ] **Step 2: Run e2e test**

Run: `cargo test --test test_e2e -- --test-threads=1`
Expected: Test passes

- [ ] **Step 3: Commit**

```bash
git add tests/test_e2e.rs
git commit -m "feat: add end-to-end agent workflow test"
```

---

### Task 17: Skill document

**Files:**
- Create: `skill/agent-procs.md`

- [ ] **Step 1: Write the skill**

Create `skill/agent-procs.md`:

````markdown
---
name: agent-procs
description: Manage concurrent processes (dev servers, databases, builds). Use when you need to run multiple processes simultaneously, wait for services to be ready, or monitor running processes.
---

# agent-procs — Concurrent Process Runner

Use `agent-procs` when you need to run and manage multiple processes concurrently.

## When to use

- Starting a dev stack (database + API + frontend)
- Running build/test in background while editing
- Any time you need 2+ processes running at once

Do NOT use for single commands you just need output from — use Bash directly.

## Config-based startup (preferred for projects)

If the project has an `agent-procs.yaml`:

```bash
agent-procs up                    # start everything, waits for ready signals
agent-procs status                # check all processes
agent-procs logs --all --tail 30  # recent output from everything
agent-procs down                  # stop everything
```

## Ad-hoc processes

```bash
agent-procs run "npm run dev" --name webserver
agent-procs wait webserver --until "Listening on" --timeout 30
agent-procs run "cargo watch -x test" --name tests
agent-procs status
```

## Checking output

```bash
agent-procs logs webserver --tail 50      # last 50 lines
agent-procs logs webserver --stderr       # stderr only
agent-procs logs --all --tail 30          # all processes interleaved
```

## Waiting

```bash
agent-procs wait server --until "ready" --timeout 30   # wait for pattern
agent-procs wait build --exit --timeout 120             # wait for completion
```

Exit code 0 = condition met, 1 = timeout.

## Stopping

```bash
agent-procs stop webserver    # stop one process
agent-procs stop-all          # stop everything
agent-procs down              # stop everything (config-managed)
```

## Common recipes

**Start a full dev stack:**
```bash
agent-procs up                              # uses agent-procs.yaml
agent-procs status                          # verify everything is running
# ... do your work ...
agent-procs down                            # clean up
```

**Run tests in background while editing:**
```bash
agent-procs run "cargo watch -x test" --name tests
# ... make edits ...
agent-procs logs tests --tail 20            # check test results
agent-procs stop tests
```

**Watch build output for errors:**
```bash
agent-procs run "npm run build --watch" --name build
agent-procs wait build --until "error" --timeout 60
# if exit code 0: error was found — check logs
# if exit code 1: timeout, no errors seen
```

## Reading status output

```bash
agent-procs status
# NAME        PID    STATE    EXIT  UPTIME
# webserver   12345  running  -     2m30s    ← healthy
# tests       12348  exited   0     -        ← completed successfully
# build       12350  exited   1     -        ← FAILED — check logs
```

If a process shows `exited` with a non-zero exit code, check its logs:
```bash
agent-procs logs build --tail 50
agent-procs logs build --stderr
```

## Rules

1. **Always use --timeout on waits** — never wait without a timeout
2. **Check status after starting** — verify processes are running
3. **Clean up when done** — run `down` or `stop-all`
4. **Use --json for parsing** — `agent-procs status --json` for structured output
5. **Check exit codes** — non-zero means something failed
6. **Stop all before starting fresh** — run `stop-all` before a new session to avoid conflicts
````

- [ ] **Step 2: Commit**

```bash
git add skill/agent-procs.md
git commit -m "feat: add agent skill document"
```

---

### Known Limitations (v1)

These are acknowledged trade-offs, not bugs:

- **`logs --all` is concatenated per-process, not truly interleaved by timestamp.** True interleaving would require timestamps on each log line. The current output shows all lines from process A, then all from B. Sufficient for agent use.
- **`logs --follow` is deferred.** The flag is accepted but prints a warning and falls back to `--tail`. Implementation would require streaming from the daemon's broadcast channel.
- **`up --only` does not auto-include dependencies.** Running `--only api` when `api` depends on `db` will skip `db`. The user must include all needed processes.
- **`env` in config uses shell prefix approach.** Environment variables are prepended as `KEY=VALUE cmd` in the shell. This works for typical values but may break with special characters. A future version could add `env` to the protocol.
- **`test_log_rotation.rs` is listed in the file structure but not implemented.** Log rotation is handled in `log_writer.rs` but has no dedicated integration test. The implementer should add one if time permits.

---

### Task 18: Final cleanup

- [ ] **Step 1: Run full test suite**

Run: `cargo test -- --test-threads=1`
Expected: All tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings. Fix any issues.

- [ ] **Step 3: Build release binary**

Run: `cargo build --release`
Expected: Builds successfully

- [ ] **Step 4: Smoke test**

```bash
./target/release/agent-procs run "echo hello" --name test1
./target/release/agent-procs status
./target/release/agent-procs logs test1 --tail 5
./target/release/agent-procs stop-all
```
Expected: All commands work

- [ ] **Step 5: Commit**

Stage only the specific files that were modified during clippy fixes, then:

```bash
git commit -m "chore: clippy fixes and final cleanup"
```
