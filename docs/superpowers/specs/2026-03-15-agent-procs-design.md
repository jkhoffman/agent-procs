# agent-procs Design Spec

A concurrent process runner for AI agents. Like `mprocs`, but with a CLI-first interface designed for LLMs to use via shell commands.

## Problem

LLM agents frequently need to run multiple processes concurrently — dev servers, databases, test watchers, build tools. Existing tools like `mprocs` are TUI-first (wrong abstraction for agents), and raw background shell jobs (`&`, `nohup`) give agents no clean way to monitor output, wait for readiness, or manage lifecycle.

`agent-procs` provides a headless-first process runner with a CLI that agents interact with naturally through shell commands, plus an optional TUI for human observation.

## Architecture

Daemon + CLI architecture. The daemon owns all child processes and captures their output. CLI commands are thin clients that communicate with the daemon over a Unix domain socket.

```
agent-procs run "cmd"  ──▶  daemon (owns processes, writes logs)
agent-procs status     ──▶       ▲
agent-procs logs p1    ──▶       │ Unix domain socket
agent-procs stop p1    ──▶       │
agent-procs ui         ──▶  TUI client (connects to same socket)
```

### Components

**Daemon** — background process auto-spawned on first `run` or `up`. Manages child process lifecycle, captures output to disk, handles wait subscriptions, serves client requests.

**CLI** — thin client. Every command connects to the daemon via the socket, sends a JSON request, receives a response. If no daemon exists, `run` and `up` auto-spawn one.

**TUI** — optional human observation client via `agent-procs ui`. Connects to the same daemon socket. Not a priority for v1.

### State Directories

Follows the XDG Base Directory Specification:

- **Socket + PID** → `$XDG_RUNTIME_DIR/agent-procs/sessions/<id>/` (falls back to `/tmp/agent-procs-$UID/` when unset, common on macOS)
- **Logs + state** → `$XDG_STATE_HOME/agent-procs/sessions/<id>/` (defaults to `~/.local/state/agent-procs/`)
- **Config** → `$XDG_CONFIG_HOME/agent-procs/` (defaults to `~/.config/agent-procs/`)

```
$XDG_RUNTIME_DIR/agent-procs/sessions/<id>/
  socket              # Unix domain socket
  daemon.pid          # for stale detection

$XDG_STATE_HOME/agent-procs/sessions/<id>/
  logs/
    webserver.stdout   # per-process output
    webserver.stderr
    tests.stdout
    tests.stderr
  state.json          # process table (names, PIDs, status, exit codes)
```

### Socket Protocol

- Unix domain socket, newline-delimited JSON messages
- Request/response for commands: `run`, `stop`, `status`, `restart`
- Streaming mode for `logs --follow` and `wait` (server pushes lines until condition met or timeout)

## CLI Interface

### Process Lifecycle

```bash
agent-procs run "npm run dev" --name webserver    # spawn process, returns ID/name
agent-procs run "cargo test" --name tests         # add another
agent-procs stop webserver                         # SIGTERM, then SIGKILL after timeout
agent-procs stop-all                               # tear down all processes
agent-procs restart webserver                      # stop + start with same command
```

### Observation

```bash
agent-procs status                                 # table of all processes
agent-procs logs webserver --tail 50               # last 50 lines (default: 100)
agent-procs logs webserver --follow --timeout 10   # stream output for up to 10s
agent-procs logs webserver --stderr                # stderr only
agent-procs logs --all --tail 30                   # combined interleaved output with name prefixes
```

### Waiting

```bash
agent-procs wait webserver --until "Listening on"  # block until pattern appears in output
agent-procs wait tests --exit                      # block until process exits
agent-procs wait tests --exit --timeout 60         # with timeout
```

All wait commands return exit code 0 on match, 1 on timeout.

### Exit Code Conventions

All commands return 0 on success. Non-zero on error, with a message to stderr. Specific codes:
- `0` — success (or pattern matched for `wait`)
- `1` — timeout (for `wait`), or general error
- `2` — target process not found (for `stop`, `logs`, `wait` with an unknown name/ID)

### Session Management

A session is created implicitly on first `run` or `up`. By default, a session named `default` is used. Named sessions can be specified with `--session <name>` on any command to run isolated groups of processes.

Session IDs are derived from the session name (e.g., `default`, `frontend`, `backend`). The `default` session is used when `--session` is omitted.

```bash
agent-procs session list                           # list active sessions
agent-procs session clean                          # remove stale sessions
```

### Config-Based Startup

```bash
agent-procs up                                     # start everything per config
agent-procs up --only api,web                      # start a subset
agent-procs down                                   # stop everything started by up
```

### Output Format

Plain text by default, `--json` flag for structured output.

```
NAME        PID    STATE    EXIT  UPTIME
webserver   12345  running  -     2m30s
tests       12348  exited   0     -
build       12350  exited   1     -
```

With `--json`:
```json
[{"name":"webserver","pid":12345,"state":"running","exit_code":null,"uptime_secs":150}]
```

### Combined Log View

```bash
agent-procs logs --all --tail 30
```

```
[db]  ready to accept connections
[api] Connected to database
[api] Listening on :8080
[web] ready on http://localhost:3000
```

### Design Choices

- `run` returns immediately after spawning (non-blocking) — agent can `wait` if needed
- All process references accept either name or auto-assigned short ID
- `--follow` has a default timeout of 30s (override with `--timeout`). Omitting `--timeout` is not an error — it uses the default.
- `wait --until` does substring match by default, `--regex` flag for regex patterns

## Project Config File

Auto-discovered from the current directory (walks up parents like `.gitignore`), or explicit with `--config path`.

### `agent-procs.yaml`

```yaml
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

  web:
    cmd: npm run dev
    cwd: ./frontend
    ready: "ready on http://localhost:3000"
    depends_on: [api]
```

### Config Fields Per Process

| Field | Required | Description |
|-------|----------|-------------|
| `cmd` | yes | The shell command to run |
| `cwd` | no | Working directory (relative to config file location) |
| `env` | no | Environment variables (key-value map) |
| `ready` | no | Substring pattern — `up` waits for this before starting dependents |
| `depends_on` | no | List of process names that must be ready first |

### Startup Behavior

`agent-procs up` reads the config, topologically sorts by `depends_on`, and starts processes in order. For each process with a `ready` pattern, it blocks until the pattern appears in output before starting dependents. Processes with no dependencies start concurrently.

## Daemon Internals

### Lifecycle

- Auto-spawned on first `run` or `up` via double-fork (detaches from agent's shell)
- Writes PID to `$XDG_RUNTIME_DIR/agent-procs/sessions/<id>/daemon.pid`
- Exits when all processes have exited and no clients are connected, or after a configurable idle timeout
- `stop-all` / `down` kills all children, then daemon exits

### Process Management

- Spawns children with stdout/stderr piped to the daemon
- Writes output to log files in real-time (line-buffered)
- Reaps zombies via `SIGCHLD` handler, updates `state.json`
- Sends `SIGTERM` on stop, escalates to `SIGKILL` after 10s (configurable)

### Wait Engine

- Maintains a set of active wait subscriptions (pattern + client connection)
- As output lines arrive from child processes, checks against all active patterns
- When a pattern matches or target process exits, unblocks the waiting client
- Timeouts handled client-side — client sends timeout value with the wait request, daemon respects it

### Stale Session Detection

- On connect, CLI checks `daemon.pid` — if the PID is dead, cleans up socket and state directory
- `session clean` does this across all sessions

### Log Size Limits

- Default max 50MB per process log file
- When limit is reached, rotates: current log → `<name>.stdout.1`, starts fresh
- Configurable via CLI flag or config

## Optional TUI

Human observation interface. Not a priority for v1 — the socket protocol supports it from day one, so it can be added later without architectural changes.

### Layout

```
┌─ Processes ──────┬─ Output: webserver ──────────────────────┐
│                  │                                           │
│ ● webserver      │  [14:23:01] Server starting              │
│   api            │  [14:23:02] Listening on :3000           │
│   db             │  [14:23:05] GET / 200 12ms               │
│ ✗ build (1)      │  [14:23:06] GET /api 200 8ms             │
│                  │                                           │
├──────────────────┴──────────────────────────────────────────┤
│ 4 processes (3 running, 1 exited)          session: default │
└─────────────────────────────────────────────────────────────┘
```

### Implementation

- `agent-procs ui` connects to daemon socket, subscribes to all output streams
- Built with `ratatui`
- Keyboard: `j/k` or arrows to select process, `q` to quit, `r` to restart, `x` to stop

## Skill Layer

A skill (prompt document) that teaches LLM agents how to use `agent-procs` effectively. Distributed alongside the CLI, usable as a Claude Code plugin skill or reference prompt for other agent frameworks.

### Skill Covers

- When to use `agent-procs` vs running a single process directly
- The `up`/`down` pattern for projects with config files
- The `run` + `wait --until` pattern for ad-hoc processes
- How to read `status` output and react to failures
- Common recipes: start a dev stack, run tests in background, watch build for errors
- Gotchas: always use `--timeout` on waits, check exit codes, `stop-all` before starting fresh

### Skill Does Not

- Contain agent-specific logic — works with any agent that can run shell commands
- Auto-invoke — the agent (or its skill system) decides when process management is needed

## Technology

- **Language:** Rust
- **Process management:** `tokio` for async I/O, `nix` crate for Unix process control
- **Socket:** `tokio` Unix domain socket with newline-delimited JSON
- **TUI (v2):** `ratatui`
- **Config parsing:** `serde` + `serde_yaml`
- **CLI argument parsing:** `clap`

## Explicitly Out of Scope (v1)

- TUI implementation (design the socket protocol to support it, but don't build the TUI)
- Restart policies / service supervision
- Sending arbitrary signals (stop is sufficient)
- stdin forwarding to processes
- Process resource limits (cgroups)
- Remote/networked daemon access
- Windows support
