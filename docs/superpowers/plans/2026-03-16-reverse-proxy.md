# Reverse Proxy Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an opt-in embedded reverse proxy so agent-managed processes get stable named `.localhost` URLs.

**Architecture:** Extends the existing daemon with optional HTTP proxy alongside the Unix socket listener. Port assignment + env injection in process_manager. New `proxy.rs` module for HTTP routing. All opt-in via `proxy: true` config or `--proxy` CLI flag.

**Tech Stack:** hyper + hyper-util for HTTP, http crate for types. Existing tokio runtime.

**Spec:** `docs/superpowers/specs/2026-03-16-reverse-proxy-design.md`

**Deferred to follow-up:** WebSocket upgrade proxying (needed for HMR in Vite/Next.js/webpack). The spec lists this as a requirement but notes `tokio-tungstenite` can be deferred "if it inflates scope." We defer it here because: (1) the proxy's `.with_upgrades()` call lays the groundwork, (2) HTTP proxying covers the core value proposition, and (3) WebSocket proxying adds ~100-150 lines and a new dependency that deserve their own focused task. A follow-up plan should add `tokio-tungstenite`, detect `Connection: Upgrade` headers, and establish bidirectional streaming.

---

## File Structure

| File | Role | Action |
|------|------|--------|
| `src/protocol.rs` | Request/response types | Modify: add `port`/`url` fields, `EnableProxy` request |
| `src/config.rs` | YAML config parsing | Modify: add `proxy`, `proxy_port`, per-process `port` |
| `src/daemon/process_manager.rs` | Process lifecycle + port assignment | Modify: port tracking, auto-assign, bind-test, env injection, DNS validation |
| `src/daemon/server.rs` | Daemon main loop + request dispatch | Modify: proxy state in `DaemonState`, `EnableProxy` handler, start proxy listener |
| `src/daemon/proxy.rs` | HTTP reverse proxy | Create: listener, subdomain routing, request forwarding |
| `src/daemon/mod.rs` | Module declarations | Modify: add `pub mod proxy` |
| `src/cli/run.rs` | `run` CLI command | Modify: `--port` and `--proxy` flags |
| `src/cli/up.rs` | `up` CLI command | Modify: pass port config, send `EnableProxy` |
| `src/cli/status.rs` | `status` CLI command | Modify: URL column |
| `src/main.rs` | CLI argument definitions | Modify: add `--port` and `--proxy` to `Run`, `--proxy` to `Up` |
| `Cargo.toml` | Dependencies | Modify: add `hyper`, `hyper-util`, `http` |
| `tests/test_protocol.rs` | Protocol roundtrip tests | Modify: update existing, add new |
| `tests/test_config.rs` | Config parsing tests | Modify: add proxy/port field tests |
| `tests/test_port_assignment.rs` | Port assignment unit tests | Create |
| `tests/test_proxy.rs` | Proxy E2E tests | Create |

---

## Chunk 1: Protocol and Config Types

### Task 1: Add port and url fields to protocol response types

**Files:**
- Modify: `src/protocol.rs:42-84`
- Modify: `tests/test_protocol.rs`

- [ ] **Step 1: Write failing test — RunOk with port and url**

Add to `tests/test_protocol.rs`:

```rust
#[test]
fn test_run_ok_with_port_and_url() {
    let resp = Response::RunOk {
        name: "api".into(),
        id: "p1".into(),
        pid: 12345,
        port: Some(4000),
        url: Some("http://api.localhost:9090".into()),
    };
    let json = serde_json::to_string(&resp).unwrap();
    let decoded: Response = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, decoded);
}

#[test]
fn test_run_ok_without_port() {
    let resp = Response::RunOk {
        name: "api".into(),
        id: "p1".into(),
        pid: 12345,
        port: None,
        url: None,
    };
    let json = serde_json::to_string(&resp).unwrap();
    let decoded: Response = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, decoded);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_run_ok_with_port_and_url -- --nocapture`
Expected: FAIL — `RunOk` doesn't have `port` or `url` fields

- [ ] **Step 3: Add port and url to RunOk and ProcessInfo**

In `src/protocol.rs`, add fields to `Response::RunOk`:

```rust
RunOk {
    name: String,
    id: String,
    pid: u32,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    url: Option<String>,
},
```

Add fields to `ProcessInfo`:

```rust
pub struct ProcessInfo {
    pub name: String,
    pub id: String,
    pub pid: u32,
    pub state: ProcessState,
    pub exit_code: Option<i32>,
    pub uptime_secs: Option<u64>,
    pub command: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub url: Option<String>,
}
```

- [ ] **Step 4: Fix all compilation errors**

Update every place that constructs `RunOk` or `ProcessInfo` to include the new fields as `None`:

- `src/daemon/process_manager.rs:152` — `Response::RunOk { name, id, pid }` → add `port: None, url: None`
- `src/daemon/process_manager.rs:240-257` — `ProcessInfo { ... }` → add `port: None, url: None`
- `src/cli/run.rs:11` — `Ok(Response::RunOk { name, id, pid })` → add `port: _, url: _` or use `..`
- `src/cli/up.rs:84` — `Ok(Response::RunOk { name, id, pid, .. })` → already uses `..`, no change
- `tests/test_protocol.rs:22-30` — `ProcessInfo { ... }` in test → add `port: None, url: None`

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test test_run_ok -- --nocapture`
Expected: PASS for both new tests

Run: `cargo test` to make sure nothing else broke
Expected: all existing tests pass

- [ ] **Step 6: Commit**

```bash
git add src/protocol.rs src/daemon/process_manager.rs src/cli/run.rs tests/test_protocol.rs
git commit -m "feat: add port and url fields to RunOk and ProcessInfo"
```

### Task 2: Add EnableProxy request and response to protocol

**Files:**
- Modify: `src/protocol.rs:6-40`
- Modify: `src/daemon/server.rs:163-260`
- Modify: `tests/test_protocol.rs`

- [ ] **Step 1: Write failing test — EnableProxy roundtrip**

Add to `tests/test_protocol.rs`:

```rust
#[test]
fn test_enable_proxy_request_roundtrip() {
    let req = Request::EnableProxy {
        proxy_port: Some(9090),
    };
    let json = serde_json::to_string(&req).unwrap();
    let decoded: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn test_enable_proxy_request_no_port() {
    let req = Request::EnableProxy { proxy_port: None };
    let json = serde_json::to_string(&req).unwrap();
    let decoded: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, decoded);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_enable_proxy_request -- --nocapture`
Expected: FAIL — `EnableProxy` variant doesn't exist

- [ ] **Step 3: Add EnableProxy to Request enum**

In `src/protocol.rs`, add to `Request`:

```rust
EnableProxy {
    #[serde(default)]
    proxy_port: Option<u16>,
},
```

Add a stub match arm in `src/daemon/server.rs` `handle_request()` so it compiles:

```rust
Request::EnableProxy { proxy_port } => {
    // TODO: implement in Task 8
    Response::Error {
        code: 1,
        message: "proxy not yet implemented".into(),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test test_enable_proxy_request -- --nocapture`
Expected: PASS

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add src/protocol.rs src/daemon/server.rs tests/test_protocol.rs
git commit -m "feat: add EnableProxy request type to protocol"
```

### Task 3: Add port to Request::Run

**Files:**
- Modify: `src/protocol.rs:7-13`
- Modify: `tests/test_protocol.rs`

- [ ] **Step 1: Write failing test — Run request with port**

Add to `tests/test_protocol.rs`:

```rust
#[test]
fn test_run_request_with_port() {
    let req = Request::Run {
        command: "node server.js".into(),
        name: Some("api".into()),
        cwd: None,
        env: None,
        port: Some(4000),
    };
    let json = serde_json::to_string(&req).unwrap();
    let decoded: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, decoded);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_run_request_with_port -- --nocapture`
Expected: FAIL — no `port` field on `Run`

- [ ] **Step 3: Add port field to Request::Run**

In `src/protocol.rs`, add to `Run` variant:

```rust
Run {
    command: String,
    name: Option<String>,
    cwd: Option<String>,
    #[serde(default)]
    env: Option<HashMap<String, String>>,
    #[serde(default)]
    port: Option<u16>,
},
```

Fix all construction sites:

- `src/cli/run.rs:4-9` — add `port: None` to the `Request::Run` construction
- `src/cli/up.rs:66-71` — add `port: None` to the `Request::Run` construction

Fix all destructuring sites:

- `src/daemon/server.rs:165-170` — add `port` to the `Request::Run { command, name, cwd, env }` destructure (ignore it for now: `port: _`)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test test_run_request_with_port -- --nocapture`
Expected: PASS

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add src/protocol.rs src/cli/run.rs src/cli/up.rs src/daemon/server.rs tests/test_protocol.rs
git commit -m "feat: add port field to Request::Run"
```

### Task 4: Add proxy fields to config

**Files:**
- Modify: `src/config.rs:7-22`
- Modify: `tests/test_config.rs`

- [ ] **Step 1: Write failing tests — config with proxy and port**

Add to `tests/test_config.rs`:

```rust
#[test]
fn test_parse_config_with_proxy() {
    let yaml = r#"
proxy: true
proxy_port: 9095
processes:
  api:
    cmd: node server.js
    port: 3001
  web:
    cmd: next dev
"#;
    let config: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.proxy, Some(true));
    assert_eq!(config.proxy_port, Some(9095));
    assert_eq!(config.processes["api"].port, Some(3001));
    assert_eq!(config.processes["web"].port, None);
}

#[test]
fn test_parse_config_without_proxy_is_backward_compatible() {
    let yaml = "processes:\n  web:\n    cmd: npm run dev\n";
    let config: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.proxy, None);
    assert_eq!(config.proxy_port, None);
    assert_eq!(config.processes["web"].port, None);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test test_parse_config_with_proxy -- --nocapture`
Expected: FAIL — no `proxy`/`proxy_port` field on `ProjectConfig`, no `port` on `ProcessDef`

- [ ] **Step 3: Add fields to config structs**

In `src/config.rs`, update `ProjectConfig`:

```rust
#[derive(Debug, Deserialize)]
pub struct ProjectConfig {
    pub session: Option<String>,
    #[serde(default)]
    pub proxy: Option<bool>,
    #[serde(default)]
    pub proxy_port: Option<u16>,
    pub processes: HashMap<String, ProcessDef>,
}
```

Update `ProcessDef`:

```rust
#[derive(Debug, Deserialize)]
pub struct ProcessDef {
    pub cmd: String,
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub ready: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub port: Option<u16>,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test test_parse_config -- --nocapture`
Expected: PASS

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add src/config.rs tests/test_config.rs
git commit -m "feat: add proxy and port fields to config"
```

---

## Chunk 2: Port Assignment and DNS Validation

### Task 5: Add port tracking and injection to ProcessManager

**Files:**
- Modify: `src/daemon/process_manager.rs:13-23,43-153`
- Create: `tests/test_port_assignment.rs`

- [ ] **Step 1: Write failing test — port stored in ManagedProcess and returned in RunOk**

Create `tests/test_port_assignment.rs`:

```rust
use agent_procs::protocol::*;
use std::collections::HashMap;

#[test]
fn test_run_request_with_port_returns_port_in_response() {
    // Verify that when we serialize a RunOk with port info, it roundtrips correctly
    let resp = Response::RunOk {
        name: "api".into(),
        id: "p1".into(),
        pid: 1234,
        port: Some(4000),
        url: Some("http://127.0.0.1:4000".into()),
    };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("4000"));
    assert!(json.contains("http://127.0.0.1:4000"));
}
```

- [ ] **Step 2: Run test to verify it passes (sanity check — protocol fields already exist)**

Run: `cargo test test_run_request_with_port_returns -- --nocapture`
Expected: PASS (protocol fields from Task 1)

- [ ] **Step 3: Add port field to ManagedProcess**

In `src/daemon/process_manager.rs`, update `ManagedProcess`:

```rust
pub struct ManagedProcess {
    pub name: String,
    pub id: String,
    pub command: String,
    pub cwd: Option<String>,
    pub env: HashMap<String, String>,
    pub child: Option<Child>,
    pub pid: u32,
    pub started_at: Instant,
    pub exit_code: Option<i32>,
    pub port: Option<u16>,
}
```

Add `port: None` to the `ManagedProcess` construction in `spawn_process()` (line ~139).

- [ ] **Step 4: Update spawn_process to accept and handle port**

Change `spawn_process` signature to accept port:

```rust
pub async fn spawn_process(
    &mut self,
    command: &str,
    name: Option<String>,
    cwd: Option<&str>,
    env: Option<&HashMap<String, String>>,
    port: Option<u16>,
) -> Response {
```

Before the `cmd.envs()` call, inject PORT and HOST if port is set:

```rust
// Inject PORT and HOST env vars if port is specified
let mut merged_env: HashMap<String, String> = HashMap::new();
if let Some(p) = port {
    merged_env.insert("PORT".to_string(), p.to_string());
    merged_env.insert("HOST".to_string(), "127.0.0.1".to_string());
}
// User-provided env overrides injected values
if let Some(env_vars) = env {
    merged_env.extend(env_vars.clone());
}
if !merged_env.is_empty() {
    cmd.envs(&merged_env);
}
```

Store port in `ManagedProcess`:
```rust
port,
```

Update the `Response::RunOk` return to include port and url:

```rust
let url = port.map(|p| format!("http://127.0.0.1:{}", p));
Response::RunOk {
    name,
    id,
    pid,
    port,
    url,
}
```

- [ ] **Step 5: Update status() to include port and url**

In `status()`, update the `ProcessInfo` construction:

```rust
port: p.port,
url: p.port.map(|port| format!("http://127.0.0.1:{}", port)),
```

Note: The proxy URL format (`http://<name>.localhost:<proxy_port>`) will be added in Task 8 when the proxy state is available. For now, use the direct URL.

- [ ] **Step 6: Fix all callers of spawn_process**

Every call to `spawn_process` now needs the extra `port` argument:

- `src/daemon/server.rs:165-177` — in `handle_request`, destructure `port` from `Request::Run` and pass it:
  ```rust
  Request::Run { command, name, cwd, env, port } => {
      state.lock().await.process_manager
          .spawn_process(&command, name, cwd.as_deref(), env.as_ref(), port)
          .await
  }
  ```

- `src/daemon/process_manager.rs` in `restart_process()` — save and reuse the port:
  ```rust
  pub async fn restart_process(&mut self, target: &str) -> Response {
      let (command, name, cwd, env, port) = match self.find(target) {
          Some(p) => (
              p.command.clone(),
              p.name.clone(),
              p.cwd.clone(),
              p.env.clone(),
              p.port,
          ),
          None => { /* existing error handling */ }
      };
      self.stop_process(target).await;
      self.processes.remove(&name);
      let env = if env.is_empty() { None } else { Some(env) };
      self.spawn_process(&command, Some(name), cwd.as_deref(), env.as_ref(), port)
          .await
  }
  ```

- [ ] **Step 7: Run all tests**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 8: Commit**

```bash
git add src/daemon/process_manager.rs src/daemon/server.rs tests/test_port_assignment.rs
git commit -m "feat: port tracking and PORT/HOST env injection in process manager"
```

### Task 6: Add port auto-assignment with bind-test

**Files:**
- Modify: `src/daemon/process_manager.rs`

- [ ] **Step 1: Add auto-assign method to ProcessManager**

Add a `proxy_enabled` field to `ProcessManager`:

```rust
pub struct ProcessManager {
    processes: HashMap<String, ManagedProcess>,
    id_counter: IdCounter,
    session: String,
    pub output_tx: broadcast::Sender<OutputLine>,
    pub proxy_enabled: bool,
    next_auto_port: u16,
}
```

Initialize in `new()`:
```rust
proxy_enabled: false,
next_auto_port: 4000,
```

Add the auto-assign method:

```rust
const AUTO_PORT_MIN: u16 = 4000;
const AUTO_PORT_MAX: u16 = 4999;

fn auto_assign_port(&mut self) -> Result<u16, String> {
    let start = self.next_auto_port;
    loop {
        let port = self.next_auto_port;
        self.next_auto_port += 1;
        if self.next_auto_port > AUTO_PORT_MAX {
            self.next_auto_port = AUTO_PORT_MIN;
        }

        // Check not already assigned to another process
        if self.processes.values().any(|p| p.port == Some(port)) {
            if self.next_auto_port == start {
                return Err("No available ports in range 4000-4999".into());
            }
            continue;
        }

        // Bind-test: verify the port is available on the OS
        match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(_listener) => return Ok(port), // listener drops, releasing the port
            Err(_) => {
                if self.next_auto_port == start {
                    return Err("No available ports in range 4000-4999".into());
                }
                continue;
            }
        }
    }
}
```

- [ ] **Step 2: Integrate auto-assignment into spawn_process**

At the top of `spawn_process`, after name validation, resolve the port:

```rust
// Resolve port: use explicit port, or auto-assign if proxy is enabled
let resolved_port = if let Some(p) = port {
    Some(p)
} else if self.proxy_enabled {
    match self.auto_assign_port() {
        Ok(p) => Some(p),
        Err(e) => return Response::Error { code: 1, message: e },
    }
} else {
    None
};
```

Then use `resolved_port` instead of `port` in the rest of the method (env injection, ManagedProcess construction, RunOk response).

- [ ] **Step 3: Run all tests**

Run: `cargo test`
Expected: all tests pass (proxy_enabled defaults to false, so no auto-assignment in existing tests)

- [ ] **Step 4: Commit**

```bash
git add src/daemon/process_manager.rs
git commit -m "feat: port auto-assignment with bind-test (4000-4999 range)"
```

### Task 7: Add DNS name validation when proxy is active

**Files:**
- Modify: `src/daemon/process_manager.rs`
- Create: `tests/test_port_assignment.rs` (add tests)

- [ ] **Step 1: Write failing test**

Add to `tests/test_port_assignment.rs`:

```rust
use agent_procs::daemon::process_manager::is_valid_dns_label;

#[test]
fn test_valid_dns_labels() {
    assert!(is_valid_dns_label("api"));
    assert!(is_valid_dns_label("my-service"));
    assert!(is_valid_dns_label("web123"));
    assert!(is_valid_dns_label("a"));
}

#[test]
fn test_invalid_dns_labels() {
    assert!(!is_valid_dns_label(""));
    assert!(!is_valid_dns_label("-api"));
    assert!(!is_valid_dns_label("api-"));
    assert!(!is_valid_dns_label("my service"));
    assert!(!is_valid_dns_label("API"));
    assert!(!is_valid_dns_label("my_service"));
    assert!(!is_valid_dns_label(&"a".repeat(64)));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_valid_dns_labels -- --nocapture`
Expected: FAIL — `is_valid_dns_label` doesn't exist

- [ ] **Step 3: Implement is_valid_dns_label**

Add to `src/daemon/process_manager.rs` as a public function:

```rust
pub fn is_valid_dns_label(name: &str) -> bool {
    if name.is_empty() || name.len() > 63 {
        return false;
    }
    if name.starts_with('-') || name.ends_with('-') {
        return false;
    }
    name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}
```

- [ ] **Step 4: Use DNS validation in spawn_process when proxy is active**

In `spawn_process`, after the existing path traversal check, add:

```rust
// Stricter DNS label validation when proxy is active
if self.proxy_enabled && !is_valid_dns_label(&name) {
    return Response::Error {
        code: 1,
        message: format!(
            "invalid process name for proxy: '{}' (must be lowercase alphanumeric/hyphens, max 63 chars, no leading/trailing hyphens)",
            name
        ),
    };
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test test_valid_dns -- --nocapture`
Run: `cargo test test_invalid_dns -- --nocapture`
Expected: PASS

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add src/daemon/process_manager.rs tests/test_port_assignment.rs
git commit -m "feat: DNS label validation for process names when proxy is active"
```

---

## Chunk 3: CLI Changes

### Task 8: Add --port and --proxy flags to run command

**Files:**
- Modify: `src/main.rs:59-66`
- Modify: `src/cli/run.rs`

- [ ] **Step 1: Add CLI flags to Run in main.rs**

In `src/main.rs`, update the `Run` variant of `Commands`:

```rust
Run {
    /// Command to execute
    command: String,
    /// Process name (auto-generated if omitted)
    #[arg(long)]
    name: Option<String>,
    /// Assign a specific port (injected as PORT env var)
    #[arg(long)]
    port: Option<u16>,
    /// Enable reverse proxy for this session
    #[arg(long)]
    proxy: bool,
},
```

Update the match arm (line ~180):

```rust
Commands::Run { command, name, port, proxy } => {
    agent_procs::cli::run::execute(session, &command, name, port, proxy).await
}
```

- [ ] **Step 2: Update cli/run.rs to handle port and proxy**

```rust
use crate::protocol::{Request, Response};

pub async fn execute(
    session: &str,
    command: &str,
    name: Option<String>,
    port: Option<u16>,
    proxy: bool,
) -> i32 {
    // If --proxy, send EnableProxy request first
    if proxy {
        let enable_req = Request::EnableProxy { proxy_port: None };
        match crate::cli::request(session, &enable_req, true).await {
            Ok(Response::Ok { message }) => {
                eprintln!("{}", message);
            }
            Ok(Response::Error { code, message }) => {
                eprintln!("error enabling proxy: {}", message);
                return code;
            }
            Err(e) => {
                eprintln!("error enabling proxy: {}", e);
                return 1;
            }
            _ => {}
        }
    }

    let req = Request::Run {
        command: command.into(),
        name,
        cwd: None,
        env: None,
        port,
    };
    match crate::cli::request(session, &req, true).await {
        Ok(Response::RunOk { name, id, pid, port: _, url }) => {
            match url {
                Some(u) => println!("{} (id: {}, pid: {}, {})", name, id, pid, u),
                None => println!("{} (id: {}, pid: {})", name, id, pid),
            }
            0
        }
        Ok(Response::Error { code, message }) => {
            eprintln!("error: {}", message);
            code
        }
        Ok(_) => {
            eprintln!("unexpected response");
            1
        }
        Err(e) => {
            eprintln!("error: {}", e);
            1
        }
    }
}
```

- [ ] **Step 3: Run all tests**

Run: `cargo test`
Expected: all tests pass (integration tests don't use --port or --proxy)

- [ ] **Step 4: Commit**

```bash
git add src/main.rs src/cli/run.rs
git commit -m "feat: add --port and --proxy flags to run command"
```

### Task 9: Add --proxy flag to up command and pass port from config

**Files:**
- Modify: `src/main.rs:131-140`
- Modify: `src/cli/up.rs`

- [ ] **Step 1: Add --proxy flag to Up in main.rs**

```rust
Up {
    /// Start only these processes (comma-separated)
    #[arg(long)]
    only: Option<String>,
    /// Config file path (default: auto-discover)
    #[arg(long)]
    config: Option<String>,
    /// Enable reverse proxy for this session
    #[arg(long)]
    proxy: bool,
},
```

Update the match arm:

```rust
Commands::Up { only, config, proxy } => {
    agent_procs::cli::up::execute(cli_session_ref, only.as_deref(), config.as_deref(), proxy).await
}
```

- [ ] **Step 2: Update cli/up.rs to handle proxy and port**

Update the function signature:

```rust
pub async fn execute(
    cli_session: Option<&str>,
    only: Option<&str>,
    config_path: Option<&str>,
    proxy: bool,
) -> i32 {
```

After loading the config and resolving the session, check if proxy should be enabled:

```rust
let enable_proxy = proxy || config.proxy.unwrap_or(false);

if enable_proxy {
    let proxy_port = config.proxy_port;
    let enable_req = Request::EnableProxy { proxy_port };
    match cli::request(session, &enable_req, true).await {
        Ok(Response::Ok { message }) => {
            eprintln!("{}", message);
        }
        Ok(Response::Error { code, message }) => {
            eprintln!("error enabling proxy: {}", message);
            return code;
        }
        Err(e) => {
            eprintln!("error enabling proxy: {}", e);
            return 1;
        }
        _ => {}
    }
}
```

In the request construction loop, pass the process port:

```rust
let req = Request::Run {
    command: def.cmd.clone(),
    name: Some((*name).clone()),
    cwd: resolved_cwd,
    env,
    port: def.port,
};
```

Update the output to show URL:

```rust
Ok(Response::RunOk { name, id, pid, url, .. }) => {
    match url {
        Some(u) => println!("started {} (id: {}, pid: {}, {})", name, id, pid, u),
        None => println!("started {} (id: {}, pid: {})", name, id, pid),
    }
}
```

- [ ] **Step 3: Run all tests**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add src/main.rs src/cli/up.rs
git commit -m "feat: add --proxy flag to up command, pass port from config"
```

### Task 10: Add URL column to status output

**Files:**
- Modify: `src/cli/status.rs`

- [ ] **Step 1: Update status display to show URL column when URLs exist**

```rust
pub async fn execute(session: &str, json: bool) -> i32 {
    let req = Request::Status;
    match crate::cli::request(session, &req, false).await {
        Ok(Response::Status { processes }) => {
            if json {
                match serde_json::to_string_pretty(&processes) {
                    Ok(json) => println!("{}", json),
                    Err(e) => {
                        eprintln!("error: failed to serialize status: {}", e);
                        return 1;
                    }
                }
            } else {
                let has_urls = processes.iter().any(|p| p.url.is_some());
                if has_urls {
                    println!(
                        "{:<12} {:<8} {:<10} {:<6} {:<30} UPTIME",
                        "NAME", "PID", "STATE", "EXIT", "URL"
                    );
                } else {
                    println!(
                        "{:<12} {:<8} {:<10} {:<6} UPTIME",
                        "NAME", "PID", "STATE", "EXIT"
                    );
                }
                for p in &processes {
                    let state = match p.state {
                        ProcessState::Running => "running",
                        ProcessState::Exited => "exited",
                    };
                    let exit = p.exit_code.map(|c| c.to_string()).unwrap_or("-".into());
                    let uptime = p.uptime_secs.map(format_uptime).unwrap_or("-".into());
                    if has_urls {
                        let url = p.url.as_deref().unwrap_or("-");
                        println!(
                            "{:<12} {:<8} {:<10} {:<6} {:<30} {}",
                            p.name, p.pid, state, exit, url, uptime
                        );
                    } else {
                        println!(
                            "{:<12} {:<8} {:<10} {:<6} {}",
                            p.name, p.pid, state, exit, uptime
                        );
                    }
                }
            }
            0
        }
        Ok(Response::Error { code, message }) => {
            eprintln!("error: {}", message);
            code
        }
        _ => 1,
    }
}
```

- [ ] **Step 2: Run all tests**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 3: Commit**

```bash
git add src/cli/status.rs
git commit -m "feat: show URL column in status output when URLs exist"
```

---

## Chunk 4: Reverse Proxy

### Task 11: Add dependencies to Cargo.toml

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add hyper, hyper-util, and http dependencies**

Add to `[dependencies]` in `Cargo.toml`:

```toml
hyper = { version = "1", features = ["http1", "server", "client"] }
hyper-util = { version = "0.1", features = ["tokio", "http1", "server", "client-legacy"] }
http = "1"
http-body-util = "0.1"
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check`
Expected: compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat: add hyper and http dependencies for reverse proxy"
```

### Task 12: Create proxy module with subdomain extraction and routing

**Files:**
- Create: `src/daemon/proxy.rs`
- Modify: `src/daemon/mod.rs`

- [ ] **Step 1: Write the proxy module**

Create `src/daemon/proxy.rs`.

**Important design note:** The proxy streams request and response bodies rather than buffering them in memory. This means the handler is generic over body types — requests use `Incoming` (hyper's streaming body from the client) and responses use `BoxBody` (either streamed from the backend or a small error message). The `Full<Bytes>` type is only used for error responses and the status page.

```rust
use crate::daemon::server::DaemonState;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

const PROXY_PORT_MIN: u16 = 9090;
const PROXY_PORT_MAX: u16 = 9190;

/// Find an available proxy port via bind-test.
pub fn find_available_proxy_port(explicit: Option<u16>) -> Result<u16, String> {
    if let Some(port) = explicit {
        match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(_) => return Ok(port),
            Err(_) => return Err(format!("Proxy port {} already in use", port)),
        }
    }
    for port in PROXY_PORT_MIN..=PROXY_PORT_MAX {
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    Err(format!(
        "No available proxy ports in range {}-{}",
        PROXY_PORT_MIN, PROXY_PORT_MAX
    ))
}

/// Extract the process name from the Host header.
/// "api.localhost:9090" → Some("api")
/// "sub.api.localhost:9090" → Some("api")
/// "localhost:9090" → None
pub fn extract_subdomain(host: &str) -> Option<String> {
    // Strip port
    let host_no_port = host.split(':').next().unwrap_or(host);
    // Split by dots
    let parts: Vec<&str> = host_no_port.split('.').collect();
    // "localhost" → no subdomain
    // "api.localhost" → "api"
    // "sub.api.localhost" → "api" (second to last before "localhost")
    if parts.len() < 2 {
        return None;
    }
    let last = parts[parts.len() - 1];
    if last != "localhost" {
        return None;
    }
    if parts.len() == 1 {
        return None;
    }
    // The process name is the part right before "localhost"
    let name = parts[parts.len() - 2];
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

/// Start the proxy HTTP server. Runs until the shutdown signal is received.
pub async fn start_proxy(
    proxy_port: u16,
    state: Arc<Mutex<DaemonState>>,
    shutdown: Arc<tokio::sync::Notify>,
) -> Result<(), String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], proxy_port));
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| format!("failed to bind proxy on {}: {}", addr, e))?;

    loop {
        let (stream, _) = tokio::select! {
            result = listener.accept() => match result {
                Ok(conn) => conn,
                Err(_) => continue,
            },
            _ = shutdown.notified() => break,
        };

        let state = Arc::clone(&state);
        let proxy_port = proxy_port;

        tokio::spawn(async move {
            let state = state.clone();
            let service = service_fn(move |req: Request<Incoming>| {
                let state = state.clone();
                async move {
                    handle_proxy_request(req, state, proxy_port).await
                }
            });

            let io = TokioIo::new(stream);
            if let Err(e) = http1::Builder::new()
                .serve_connection(io, service)
                .with_upgrades()
                .await
            {
                if !e.is_incomplete_message() {
                    eprintln!("proxy connection error: {}", e);
                }
            }
        });
    }

    Ok(())
}

/// Helper to wrap a string body into a BoxBody for error/status responses.
fn text_body(s: String) -> BoxBody {
    Full::new(Bytes::from(s)).map_err(|never| match never {}).boxed()
}

async fn handle_proxy_request(
    req: Request<Incoming>,
    state: Arc<Mutex<DaemonState>>,
    proxy_port: u16,
) -> Result<Response<BoxBody>, hyper::Error> {
    // Extract process name from Host header
    let host = req
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    let process_name = match extract_subdomain(host) {
        Some(name) => name,
        None => {
            // No subdomain — return status page
            return Ok(status_page(&state, proxy_port).await);
        }
    };

    // Look up process port
    let (process_port, process_exists) = {
        let s = state.lock().await;
        (
            s.process_manager.get_process_port(&process_name),
            s.process_manager.has_process(&process_name),
        )
    };

    let port = match process_port {
        Some(port) => port,
        None => {
            let body = if process_exists {
                format!("502 Bad Gateway: process \"{}\" is not running or has no port assigned", process_name)
            } else {
                format!("502 Bad Gateway: no process named \"{}\"", process_name)
            };
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(text_body(body))
                .unwrap());
        }
    };

    // Forward the request — bodies are streamed, not buffered
    let uri = format!(
        "http://127.0.0.1:{}{}",
        port,
        req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or("/")
    );

    let client = Client::builder(TokioExecutor::new()).build_http();

    let mut forwarded_req = hyper::Request::builder()
        .method(req.method().clone())
        .uri(&uri);

    // Copy headers, rewrite Host
    for (key, value) in req.headers() {
        if key == "host" {
            continue;
        }
        forwarded_req = forwarded_req.header(key, value);
    }
    forwarded_req = forwarded_req.header("host", format!("127.0.0.1:{}", port));

    // Pass the incoming body through directly — no buffering
    let forwarded_req = forwarded_req
        .body(req.into_body())
        .unwrap();

    match client.request(forwarded_req).await {
        Ok(resp) => {
            // Stream the response body through without buffering
            let (parts, body) = resp.into_parts();
            let boxed_body = body.boxed();
            Ok(Response::from_parts(parts, boxed_body))
        }
        Err(_) => {
            let body = format!(
                "502 Bad Gateway: process \"{}\" is not responding on port {}",
                process_name, port
            );
            Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(text_body(body))
                .unwrap())
        }
    }
}

async fn status_page(state: &Arc<Mutex<DaemonState>>, proxy_port: u16) -> Response<BoxBody> {
    let s = state.lock().await;
    let crate::protocol::Response::Status { processes } = s.process_manager.status_snapshot() else {
        return Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(text_body("internal error".into()))
            .unwrap();
    };

    let mut body = format!("agent-procs proxy on port {}\n\n", proxy_port);
    body.push_str("Routes:\n");
    for p in &processes {
        if let Some(port) = p.port {
            body.push_str(&format!(
                "  http://{}.localhost:{} → 127.0.0.1:{} [{}]\n",
                p.name,
                proxy_port,
                port,
                if p.state == crate::protocol::ProcessState::Running {
                    "running"
                } else {
                    "exited"
                }
            ));
        }
    }
    if processes.iter().all(|p| p.port.is_none()) {
        body.push_str("  (no processes with ports assigned)\n");
    }

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/plain")
        .body(text_body(body))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_subdomain_simple() {
        assert_eq!(extract_subdomain("api.localhost:9090"), Some("api".into()));
    }

    #[test]
    fn test_extract_subdomain_nested() {
        assert_eq!(
            extract_subdomain("tenant.api.localhost:9090"),
            Some("api".into())
        );
    }

    #[test]
    fn test_extract_subdomain_bare_localhost() {
        assert_eq!(extract_subdomain("localhost:9090"), None);
    }

    #[test]
    fn test_extract_subdomain_no_port() {
        assert_eq!(extract_subdomain("api.localhost"), Some("api".into()));
    }
}
```

- [ ] **Step 2: Add a public helper to ProcessManager for port lookup**

In `src/daemon/process_manager.rs`, add:

```rust
/// Get the port assigned to a running process by name.
/// Returns None if process not found, not running, or has no port.
pub fn get_process_port(&self, name: &str) -> Option<u16> {
    self.processes.get(name).and_then(|p| {
        if p.child.is_some() {
            p.port
        } else {
            None
        }
    })
}

/// Non-mutating status snapshot for use by proxy status page.
pub fn status_snapshot(&self) -> crate::protocol::Response {
    let mut infos: Vec<crate::protocol::ProcessInfo> = self
        .processes
        .values()
        .map(|p| crate::protocol::ProcessInfo {
            name: p.name.clone(),
            id: p.id.clone(),
            pid: p.pid,
            state: if p.child.is_some() {
                crate::protocol::ProcessState::Running
            } else {
                crate::protocol::ProcessState::Exited
            },
            exit_code: p.exit_code,
            uptime_secs: if p.child.is_some() {
                Some(p.started_at.elapsed().as_secs())
            } else {
                None
            },
            command: p.command.clone(),
            port: p.port,
            url: p.port.map(|port| format!("http://127.0.0.1:{}", port)),
        })
        .collect();
    infos.sort_by(|a, b| a.name.cmp(&b.name));
    crate::protocol::Response::Status { processes: infos }
}
```

- [ ] **Step 3: Register the module**

In `src/daemon/mod.rs`, add:

```rust
pub mod proxy;
```

- [ ] **Step 4: Run tests (unit tests in proxy module)**

Run: `cargo test daemon::proxy -- --nocapture`
Expected: PASS for subdomain extraction tests

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add src/daemon/proxy.rs src/daemon/mod.rs src/daemon/process_manager.rs
git commit -m "feat: add reverse proxy module with subdomain routing"
```

### Task 13: Wire EnableProxy into the daemon and start the proxy listener

**Files:**
- Modify: `src/daemon/server.rs`

- [ ] **Step 1: Add proxy state to DaemonState**

```rust
pub struct DaemonState {
    pub process_manager: ProcessManager,
    pub proxy_port: Option<u16>,
}
```

Update the initialization in `run()`:

```rust
let state = Arc::new(Mutex::new(DaemonState {
    process_manager: ProcessManager::new(session),
    proxy_port: None,
}));
```

- [ ] **Step 2: Implement the EnableProxy handler**

First, change the `handle_request` signature to accept `shutdown`:

```rust
async fn handle_request(
    state: &Arc<Mutex<DaemonState>>,
    shutdown: &Arc<tokio::sync::Notify>,
    request: Request,
) -> Response {
```

Then update the call site in the `tokio::spawn` block inside `run()`. The current code (server.rs line ~94) is:

```rust
let response = handle_request(&state, request).await;
```

Change it to:

```rust
let response = handle_request(&state, &shutdown, request).await;
```

The `shutdown` Arc is already cloned into the spawn block (server.rs line ~46: `let shutdown = Arc::clone(&shutdown)`), so this just passes the existing clone through.

Now replace the `EnableProxy` stub in `handle_request`:

```rust
Request::EnableProxy { proxy_port } => {
    let mut s = state.lock().await;
    if let Some(existing_port) = s.proxy_port {
        // Proxy already running — idempotent
        return Response::Ok {
            message: format!("Proxy already listening on http://localhost:{}", existing_port),
        };
    }

    // Find available port
    let port = match super::proxy::find_available_proxy_port(proxy_port) {
        Ok(p) => p,
        Err(e) => return Response::Error { code: 1, message: e },
    };

    s.proxy_port = Some(port);
    s.process_manager.proxy_enabled = true;
    drop(s); // Release lock before spawning proxy task

    // Start the proxy in a background task
    let proxy_state = Arc::clone(state);
    let proxy_shutdown = Arc::clone(shutdown);
    tokio::spawn(async move {
        if let Err(e) = super::proxy::start_proxy(port, proxy_state, proxy_shutdown).await {
            eprintln!("proxy error: {}", e);
        }
    });

    Response::Ok {
        message: format!("Proxy listening on http://localhost:{}", port),
    }
}
```

- [ ] **Step 3: Update status() and RunOk to use proxy URLs when proxy is active**

In `src/daemon/process_manager.rs`, update `status()` to accept an optional proxy_port:

The simplest approach: add a method that generates the URL with proxy awareness:

```rust
pub fn url_for_process(&self, proc: &ManagedProcess, proxy_port: Option<u16>) -> Option<String> {
    proc.port.map(|port| {
        if let Some(pp) = proxy_port {
            format!("http://{}.localhost:{}", proc.name, pp)
        } else {
            format!("http://127.0.0.1:{}", port)
        }
    })
}
```

Then in `handle_request` for `Request::Status`, pass `proxy_port`:

```rust
Request::Status => {
    let s = state.lock().await;
    let proxy_port = s.proxy_port;
    // Build status response with proxy-aware URLs
    s.process_manager.status_with_proxy(proxy_port)
}
```

Similarly, for `Request::Run`, use a single lock acquisition:

```rust
Request::Run { command, name, cwd, env, port } => {
    let mut s = state.lock().await;
    let proxy_port = s.proxy_port;
    let mut resp = s.process_manager
        .spawn_process(&command, name, cwd.as_deref(), env.as_ref(), port)
        .await;
    drop(s); // Release lock
    // Upgrade URL to proxy URL if proxy is active
    if let Response::RunOk { ref name, ref mut url, port: Some(_), .. } = resp {
        if let Some(pp) = proxy_port {
            *url = Some(format!("http://{}.localhost:{}", name, pp));
        }
    }
    resp
}
```

This avoids passing proxy_port deep into process_manager — the URL upgrade is done at the server level.

- [ ] **Step 4: Run all tests**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add src/daemon/server.rs src/daemon/process_manager.rs
git commit -m "feat: wire EnableProxy handler, start proxy listener, proxy-aware URLs"
```

---

## Chunk 5: Integration Tests

### Task 14: End-to-end proxy test

**Files:**
- Create: `tests/test_proxy.rs`

- [ ] **Step 1: Write E2E test**

Create `tests/test_proxy.rs`:

```rust
mod helpers;
use assert_cmd::Command;
use helpers::TestContext;
use std::time::Duration;

#[test]
fn test_run_with_port_shows_url() {
    let ctx = TestContext::new("t-port");
    ctx.set_env();

    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args([
            "--session",
            &ctx.session,
            "run",
            "echo 'listening' && sleep 60",
            "--name",
            "api",
            "--port",
            "4567",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("http://127.0.0.1:4567"),
        "expected URL in output: {}",
        stdout
    );

    // Status should show URL
    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "status"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("http://127.0.0.1:4567"));

    // JSON status should have port and url fields
    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "status", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"port\":"));
    assert!(stdout.contains("\"url\":"));

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_run_without_port_has_no_url() {
    let ctx = TestContext::new("t-noport");
    ctx.set_env();

    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args([
            "--session",
            &ctx.session,
            "run",
            "sleep 60",
            "--name",
            "bg",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("http://"),
        "should not have URL without --port: {}",
        stdout
    );

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

#[test]
fn test_proxy_routes_request() {
    let ctx = TestContext::new("t-proxy-route");
    ctx.set_env();

    // Start a simple HTTP server with --proxy (Python for portability)
    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args([
            "--session",
            &ctx.session,
            "run",
            "python3 -c \"from http.server import HTTPServer, BaseHTTPRequestHandler; \
             import sys; \
             class H(BaseHTTPRequestHandler): \
               def do_GET(self): self.send_response(200); self.end_headers(); self.wfile.write(b'ok') \
             ; HTTPServer(('127.0.0.1', 4500), H).serve_forever()\"",
            "--name",
            "web",
            "--port",
            "4500",
            "--proxy",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Proxy listening"),
        "expected proxy startup message: {}",
        stderr
    );

    // Give the server a moment to bind
    std::thread::sleep(Duration::from_millis(500));

    // Extract proxy port from stderr (e.g., "Proxy listening on http://localhost:9090")
    let proxy_port = stderr
        .split("localhost:")
        .nth(1)
        .and_then(|s| s.trim().parse::<u16>().ok())
        .expect("could not parse proxy port from output");

    // Make a request through the proxy
    let output = std::process::Command::new("curl")
        .args([
            "-s",
            "-H",
            &format!("Host: web.localhost:{}", proxy_port),
            &format!("http://127.0.0.1:{}/", proxy_port),
        ])
        .output();

    if let Ok(output) = output {
        let body = String::from_utf8_lossy(&output.stdout);
        assert!(
            body.contains("ok"),
            "expected 'ok' from proxied request, got: {}",
            body
        );
    }
    // If curl is not available, skip the assertion — the test still validates proxy startup

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test test_run_with_port -- --test-threads=1 --nocapture`
Run: `cargo test test_run_without_port -- --test-threads=1 --nocapture`
Run: `cargo test test_proxy_routes -- --test-threads=1 --nocapture`
Expected: PASS

- [ ] **Step 3: Run full test suite**

Run: `cargo test -- --test-threads=1`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add tests/test_proxy.rs
git commit -m "test: add integration tests for port assignment and proxy routing"
```

### Task 15: Config-driven proxy test with up

**Files:**
- Modify: `tests/test_proxy.rs`

- [ ] **Step 1: Write config-driven test**

Add to `tests/test_proxy.rs`:

```rust
use std::io::Write;
use tempfile::TempDir;

#[test]
fn test_up_with_proxy_config() {
    let ctx = TestContext::new("t-proxy-up");
    ctx.set_env();

    // Create a config file with proxy: true
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("agent-procs.yaml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    writeln!(
        f,
        r#"proxy: true
processes:
  echo-srv:
    cmd: "echo 'started' && sleep 60"
    port: 4600
"#
    )
    .unwrap();

    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args([
            "--session",
            &ctx.session,
            "up",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "up failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Proxy listening"),
        "expected proxy message: {}",
        stderr
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("echo-srv"),
        "expected process name: {}",
        stdout
    );

    // Status should show proxy URL
    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "status"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("echo-srv.localhost"),
        "expected proxy URL in status: {}",
        stdout
    );

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test test_up_with_proxy_config -- --test-threads=1 --nocapture`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add tests/test_proxy.rs
git commit -m "test: add config-driven proxy integration test"
```

### Task 16: Final verification

- [ ] **Step 1: Run full test suite**

Run: `cargo test -- --test-threads=1`
Expected: all tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: no warnings

- [ ] **Step 3: Manual smoke test**

Create a temporary `agent-procs.yaml`:
```yaml
proxy: true
processes:
  hello:
    cmd: "python3 -m http.server 0"
    port: 4100
    ready: "Serving"
```

Run: `agent-procs up`
Expected: proxy starts, hello process starts on port 4100, URL shown.

Run: `curl http://hello.localhost:<proxy_port>/`
Expected: directory listing from Python HTTP server.

Run: `agent-procs status`
Expected: URL column with proxy URL shown.

Run: `agent-procs down`
Expected: clean shutdown.

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "feat: v4 reverse proxy - complete implementation"
```
