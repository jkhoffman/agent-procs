# agent-procs v4 Design Spec: Reverse Proxy

Optional embedded reverse proxy that gives agent-managed processes stable, named `.localhost` URLs. Inspired by [portless](https://github.com/vercel-labs/portless).

## Goals

1. When enabled, processes are reachable at `http://<name>.localhost:<proxy_port>` instead of raw `http://127.0.0.1:<port>`.
2. Zero config burden — existing `agent-procs.yaml` files work unchanged. The feature is fully opt-in.
3. Agents always get a URL back from `run`/`status` when port information is available, whether or not the proxy is on.
4. Per-session proxy isolation — two sessions can both have a process named `api` without collision.

## Non-Goals

- HTTPS / TLS certificate management (can be added later without architectural changes)
- Framework auto-detection (injecting `--port` flags like portless does)
- `/etc/hosts` management
- HTTP/2
- Remote access

## Design Principles

- **Opt-in, not intrusive.** No behavior changes unless the user enables `proxy: true` or passes `--proxy`. A bare `agent-procs run -- node server.js` works exactly as today.
- **Single tool.** The agent interacts with agent-procs only. No second daemon, no external dependencies.
- **Live routing.** Routes resolve from process state at request time, not from a cached table. Restarts, stops, and new processes are reflected immediately.

## Port Assignment

Ports are assigned to processes when the user opts in via `port:` in config, `--port` on CLI, or `proxy: true` (which implies auto-assignment for processes without explicit ports).

### Auto-Assignment (when `proxy: true` and no explicit `port:`)

- Range: 4000–4999 (matches portless convention)
- Strategy: sequential starting from 4000, skipping unavailable ports
- Availability check: bind-test via `TcpListener::bind("127.0.0.1:<port>")` — if it succeeds, close immediately and assign. If not, skip to next.
- Tiny TOCTOU window between releasing the port and the child binding it. Acceptable for local dev.
- Assigned ports are stored in `ManagedProcess` and persisted in `state.json` so restarts reuse the same port.

### Explicit Port (`port: 3001` or `--port 3001`)

- Used as-is. No availability pre-check — if the port is taken, the child process fails to bind and the error appears in logs.
- Works independently of whether proxy is on or off.

### Injection

When a port is assigned (auto or explicit):

- `PORT=<assigned>` injected into child env
- `HOST=127.0.0.1` injected into child env
- User-provided env vars take precedence (if user sets `PORT` in config `env:`, that wins)

When no port is assigned (proxy off, no `port:` set): nothing is injected, process runs exactly as today.

## Protocol Changes

### `ProcessInfo` (returned by `Status`)

```rust
pub struct ProcessInfo {
    pub name: String,
    pub id: String,
    pub pid: u32,
    pub state: ProcessState,
    pub exit_code: Option<i32>,
    pub uptime_secs: Option<u64>,
    pub command: String,
    pub port: Option<u16>,        // new — assigned or explicit port
    pub url: Option<String>,      // new — proxy URL or direct URL
}
```

### `Response::RunOk` (returned when a process starts)

```rust
RunOk {
    name: String,
    id: String,
    pid: u32,
    port: Option<u16>,            // new
    url: Option<String>,          // new
}
```

URL value depends on context:

| Proxy | Port known | URL |
|-------|-----------|-----|
| off | no | `None` |
| off | yes | `Some("http://127.0.0.1:<port>")` |
| on | yes | `Some("http://<name>.localhost:<proxy_port>")` |

No new request types. Existing `Status` and `Run` requests return the enriched types.

## Reverse Proxy

### Activation

The proxy starts when:
- `proxy: true` in config and `agent-procs up` is run
- `--proxy` flag is passed to `run` or `up`

It does not start otherwise. Once started for a session, it remains running until `agent-procs down`.

### Architecture

The proxy is an HTTP server embedded in the session daemon. It runs alongside the existing Unix socket listener in the same `tokio::select!` loop.

```
Browser/agent/curl
        ↓
   TcpListener on 127.0.0.1:<proxy_port>
        ↓
   Read Host header → extract subdomain
        ↓
   Look up process by name in DaemonState
        ↓
   Forward request to 127.0.0.1:<process_port>
        ↓
   Stream response back
```

### Proxy Port

- Auto-assigned from range 9090–9190, first available (bind-tested)
- Pinnable via `proxy_port:` in config or flag
- Persisted in `state.json` so restarts reuse the same port
- Each session gets its own proxy port — no cross-session collision

### Routing Rules

- `<name>.localhost:<proxy_port>` → process named `<name>`
- `<anything>.<name>.localhost:<proxy_port>` → also routes to `<name>` (wildcard subdomain passthrough for multi-tenant apps)
- `localhost:<proxy_port>` with no subdomain → simple status page listing all routes
- Unknown subdomain → `502 Bad Gateway` with body: `No process named "<name>" is running`
- Process is stopped/crashed → `502 Bad Gateway` with body: `Process "<name>" is not running`

### Request Handling

- HTTP/1.1 requests forwarded with all headers intact
- `Host` header rewritten to `127.0.0.1:<process_port>` so backends aren't confused
- WebSocket upgrades (`Connection: Upgrade` + `Upgrade: websocket`) detected and switched to bidirectional streaming. Required for HMR in Vite/Next.js/webpack.
- No body buffering — requests and responses are streamed through
- No HTTPS (v1)

### Dependencies

- `hyper` for HTTP server and client (tokio already pulls in most of the transport layer)
- No other new dependencies

## Config Changes

Three new optional fields. Existing configs work unchanged.

```yaml
# Top-level (optional):
proxy: true            # default: false. Enables reverse proxy + auto port assignment.
proxy_port: 9095       # default: auto-assigned from 9090-9190.

# Per-process (optional):
api:
  cmd: node server.js
  port: 3001           # explicit port. Injected as PORT env var.
  ready: "Listening"

web:
  cmd: next dev
  # no port — auto-assigned when proxy is on
```

### Config Struct Changes

```rust
pub struct Config {
    pub proxy: Option<bool>,          // new
    pub proxy_port: Option<u16>,      // new
    pub processes: IndexMap<String, ProcessDef>,
}

pub struct ProcessDef {
    pub cmd: String,
    pub cwd: Option<String>,
    pub env: HashMap<String, String>,
    pub ready: Option<String>,
    pub depends_on: Vec<String>,
    pub port: Option<u16>,            // new
}
```

### Interaction Matrix

| `proxy:` | `port:` set | Behavior |
|----------|-------------|----------|
| false (default) | no | Today's behavior. No injection, no URL. |
| false | yes | Injects `PORT`, reports `http://127.0.0.1:<port>`. No proxy. |
| true | no | Auto-assigns from 4000-4999, injects `PORT`, proxy routes subdomain. |
| true | yes | Uses explicit port, injects `PORT`, proxy routes subdomain. |

## CLI Changes

### New Flags

| Flag | On | Purpose |
|------|-----|---------|
| `--port <N>` | `run` | Explicit port assignment for this process |
| `--proxy` | `run`, `up` | Enable reverse proxy for this session |

### Output Changes

`run`:
```
$ agent-procs run --name api -- node server.js
Started api (pid 12345)

$ agent-procs run --name api --port 3001 -- node server.js
Started api (pid 12345, http://127.0.0.1:3001)

$ agent-procs run --name api --proxy -- node server.js
Proxy listening on http://localhost:9090
Started api (pid 12345, http://api.localhost:9090)
```

`up` (with `proxy: true`):
```
$ agent-procs up
Proxy listening on http://localhost:9090
Started db (http://db.localhost:9090)
Waiting for db readiness... ready
Started api (http://api.localhost:9090)
Waiting for api readiness... ready
Started web (http://web.localhost:9090)
```

`status` gains a URL column when any process has a URL:
```
$ agent-procs status
NAME  STATE    URL                          PID    UPTIME
api   running  http://api.localhost:9090     12345  2m 30s
web   running  http://web.localhost:9090     12346  2m 28s
db    running  http://db.localhost:9090      12344  2m 32s
```

No new subcommands. The proxy is a daemon feature — it starts with the daemon and stops with the daemon.

## Error Handling

### Port Conflicts (Auto-Assignment)

- Port in 4000-4999 already in use → skip, try next
- Entire range exhausted → error: `"No available ports in range 4000-4999"`

### Port Conflicts (Explicit)

- Child process fails to bind → error visible in process logs. agent-procs does not pre-check.

### Proxy Port Conflicts

- Auto-assigned proxy port in use → try next in 9090-9190
- Explicit `proxy_port:` in use → daemon startup fails: `"Proxy port 9095 already in use"`

### Process Lifecycle

- Process crashes → proxy returns `502: process "api" is not running`
- Process restarts → route updates immediately (live lookup)
- New process added while proxy running → immediately routable
- Process stopped → `502` on its subdomain, other routes unaffected

### DNS

- `.localhost` subdomains resolve to `127.0.0.1` natively in Chrome, Firefox, Edge (RFC 6761)
- Safari requires `/etc/hosts` entries — documented, not automated in v1
- `curl http://api.localhost:9090/` works on RFC 6761-compliant systems

### WebSocket Upgrades

- Proxy detects `Connection: Upgrade` + `Upgrade: websocket` and switches to bidirectional streaming
- Required for HMR in Vite, Next.js, webpack dev servers

### Graceful Shutdown

- `agent-procs down` → proxy stops accepting connections, drains in-flight requests (2s timeout), daemon shuts down

## Files to Modify

| File | Changes |
|------|---------|
| `src/protocol.rs` | Add `port`, `url` fields to `ProcessInfo` and `RunOk` |
| `src/config.rs` | Add `proxy`, `proxy_port` to `Config`; add `port` to `ProcessDef` |
| `src/daemon/process_manager.rs` | Track port in `ManagedProcess`, port assignment logic, inject PORT/HOST env |
| `src/daemon/server.rs` | Add `TcpListener` in select loop, HTTP request handler, routing |
| `src/cli/run.rs` | Add `--port` and `--proxy` flags |
| `src/cli/up.rs` | Port assignment during config-driven startup |
| `src/cli/status.rs` | Display URL column |
| `Cargo.toml` | Add `hyper` dependency |

## New Files

```
src/daemon/proxy.rs    # HTTP reverse proxy: listener, router, request forwarding, WebSocket upgrade
```

## Estimated Scope

~250-300 lines of new code. No refactoring of existing code required.
