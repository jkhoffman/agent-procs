# agent-procs

Concurrent process runner for AI agents. Processes run in a background daemon and persist across CLI invocations.

## Install

```
cargo install agent-procs
```

## Quick start

```bash
# Start a process
agent-procs run "npm run dev" --name server

# Auto-restart on crash (up to 5 times)
agent-procs run "npm start" --name api --autorestart on-failure --max-restarts 5

# Restart when source files change
agent-procs run "npm run dev" --name server --watch "src/**"

# Wait for it to be ready
agent-procs wait server --until "Listening on" --timeout 30

# Check output
agent-procs logs server --tail 50

# See what's running
agent-procs status

# Stop it
agent-procs stop server

# Search for errors across all processes
agent-procs logs --all --grep "error"

# Errors since last restart, with context
agent-procs logs server --grep "error" --since restart --context 3

# Stream only matching lines live
agent-procs logs server --follow --grep "panic" --timeout 30

# JSON output for agent consumption
agent-procs logs --all --grep "error" --json
```

## Config file

Create an `agent-procs.yaml` to manage multiple processes together:

```yaml
session: myproject                          # optional — isolates this project's processes
proxy: true                                 # optional — enables reverse proxy
proxy_port: 9095                            # optional — pin proxy to a specific port

processes:
  db:
    cmd: docker compose up postgres
    ready: "ready to accept connections"
    autorestart: always
    max_restarts: 3
  api:
    cmd: ./start-api-server
    cwd: ./backend
    env:
      DATABASE_URL: postgres://localhost:5432/mydb
    ready: "Listening on :8080"
    port: 8080
    depends_on: [db]
    autorestart: on-failure
    watch:
      - "src/**"
      - "config/*"
    watch_ignore:
      - "*.generated.ts"
```

Processes start in dependency order; independent ones run concurrently.

```bash
agent-procs up                    # start all
agent-procs up --only db,api      # start specific ones
agent-procs down                  # stop all
```

### Field reference

**Per-process fields:**

| Field | Required | Description |
|-------|----------|-------------|
| `cmd` | yes | Shell command to execute |
| `cwd` | no | Working directory (relative to config file location) |
| `env` | no | Environment variables (key: value map) |
| `ready` | no | Stdout pattern that signals the process is ready |
| `depends_on` | no | List of process names that must be ready first |
| `port` | no | Port number — injected as `PORT` and `HOST=127.0.0.1` env vars |
| `autorestart` | no | Restart policy: `always`, `on-failure`, or `never` (default) |
| `max_restarts` | no | Maximum restart attempts (unlimited if omitted) |
| `restart_delay` | no | Delay between crash and restart in ms (default: 1000) |
| `watch` | no | List of glob patterns — restart process when matched files change |
| `watch_ignore` | no | Additional glob patterns to ignore (`.git`, `node_modules`, `target`, `__pycache__` always ignored) |

**Top-level fields:**

| Field | Required | Description |
|-------|----------|-------------|
| `session` | no | Session name (overridden by `--session` CLI flag) |
| `proxy` | no | Enable reverse proxy (default: false) |
| `proxy_port` | no | Pin proxy to a specific port (default: auto-assign from 9090-9190) |

## Reverse proxy

Give processes stable named URLs instead of port numbers. Opt-in via `proxy: true` in config or `--proxy` on the CLI.

```bash
$ agent-procs up
Proxy listening on http://localhost:9090
started api (http://api.localhost:9090)
started web (http://web.localhost:9090)
```

- Processes without an explicit `port` get one auto-assigned (4000-4999 range)
- `PORT` and `HOST=127.0.0.1` are injected into the process env (user env takes precedence)
- Each session gets its own proxy port, so two projects can both have `api` without conflict

Ad-hoc usage without a config file:

```bash
agent-procs run "node server.js" --name api --port 3001 --proxy
# → http://api.localhost:9090
```

## Commands

| Command | Description |
|---------|-------------|
| `run <cmd> [--name N] [--port P] [--proxy] [--autorestart MODE] [--max-restarts N] [--restart-delay MS] [--watch GLOB]... [--watch-ignore GLOB]...` | Spawn a background process |
| `stop <name>` | Stop a process |
| `stop-all` | Stop all processes |
| `restart <name>` | Restart a process |
| `status [--json]` | Show all process statuses |
| `logs <name> [--tail N] [--follow] [--stderr] [--all] [--grep PAT] [--regex] [--since SPEC] [--context N] [--json]` | View/search process output |
| `wait <name> --until <pattern> [--regex] [--timeout N]` | Wait for output pattern |
| `wait <name> --exit [--timeout N]` | Wait for process to exit |
| `up [--only X,Y] [--config path] [--proxy]` | Start from config file |
| `down` | Stop config-managed processes |
| `session list` | List active sessions |
| `session clean` | Remove stale sessions |
| `ui` | Open terminal UI |
| `completions <shell>` | Generate shell completions (bash, zsh, fish, powershell) |

## Restart policies

Automatically recover from crashes without agent intervention.

```bash
# Restart on non-zero exit, up to 5 times with 2s delay
agent-procs run "npm start" --name api \
  --autorestart on-failure --max-restarts 5 --restart-delay 2000

# Always restart (even clean exits), unlimited attempts
agent-procs run "worker" --name bg --autorestart always
```

| Mode | Behavior |
|------|----------|
| `always` | Restart on any exit |
| `on-failure` | Restart only on non-zero exit |
| `never` | Don't restart (default) |

When `max_restarts` is exhausted, the process enters `Failed` state. Manual `stop` disables auto-restart; `restart` re-enables it and resets the count.

Supervisor annotations (`[agent-procs] Restarted`, `Max restarts exhausted`) are written to disk logs and visible in `logs --tail`, `--follow`, and the TUI.

## File watch mode

Auto-restart processes when source files change.

```bash
agent-procs run "npm run dev" --name server --watch "src/**" --watch "config/*"
```

- Uses OS-native file watching (FSEvents on macOS, inotify on Linux) with 500ms debounce
- `.git`, `node_modules`, `target`, and `__pycache__` are always ignored
- Add `--watch-ignore "*.log"` for additional ignore patterns
- Watch restarts reset the restart count (they're intentional, not crashes)
- File changes can revive a `Failed` process

## Log search

Search and filter process output efficiently.

```bash
# Substring match (default)
agent-procs logs server --grep "error"

# Regex match
agent-procs logs server --grep "error|panic|warn" --regex

# Last 10 errors
agent-procs logs server --grep "error" --tail 10

# Errors since last restart, with 3 lines of context
agent-procs logs server --grep "error" --since restart --context 3

# All output since process started
agent-procs logs server --since start

# Output from the last 5 minutes
agent-procs logs server --since 5m

# JSONL output for programmatic consumption
agent-procs logs server --grep "error" --json

# Search all processes
agent-procs logs --all --grep "timeout" --since 1m
```

`--since` accepts: `Ns` (seconds), `Nm` (minutes), `Nh` (hours), `restart` (since last restart), `start` (all output). Duration estimates assume uniform log throughput.

## Sessions

Use `--session` to isolate process groups (e.g. per-project):

```bash
agent-procs --session projectA run "make serve" --name app
agent-procs --session projectB run "make serve" --name app
agent-procs --session projectA status   # only shows projectA's processes
```

## Architecture

The CLI communicates with a per-session background daemon over a Unix domain socket. The daemon manages process lifecycles, captures stdout/stderr to log files, handles wait conditions, and supervises processes with restart policies and file watchers. The daemon auto-starts on first use and exits when all processes are stopped.

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error (timeout, connection failure, unexpected response) |
| 2 | No logs found for target process |
