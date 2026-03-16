# agent-procs

Concurrent process runner for AI agents. Processes run in a background daemon and persist across CLI invocations.

## Install

```
cargo install --path .
```

## Quick start

```bash
# Start a process
agent-procs run "npm run dev" --name server

# Wait for it to be ready
agent-procs wait server --until "Listening on" --timeout 30

# Check output
agent-procs logs server --tail 50

# See what's running
agent-procs status

# Stop it
agent-procs stop server
```

## Config file

Create an `agent-procs.yaml` to manage multiple processes together:

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
```

Fields: `cmd` (required), `cwd`, `env`, `ready` (stdout pattern that signals readiness), `depends_on`.

Processes start in dependency order; independent ones run concurrently.

```bash
agent-procs up                    # start all
agent-procs up --only db,api      # start specific ones
agent-procs down                  # stop all
```

## Commands

| Command | Description |
|---------|-------------|
| `run <cmd> [--name N]` | Spawn a background process |
| `stop <name>` | Stop a process |
| `stop-all` | Stop all processes |
| `restart <name>` | Restart a process |
| `status [--json]` | Show all process statuses |
| `logs <name> [--tail N] [--follow] [--stderr] [--all]` | View process output |
| `wait <name> --until <pattern> [--regex] [--timeout N]` | Wait for output pattern |
| `wait <name> --exit [--timeout N]` | Wait for process to exit |
| `up [--only X,Y] [--config path]` | Start from config file |
| `down` | Stop config-managed processes |
| `session list` | List active sessions |
| `session clean` | Remove stale sessions |
| `ui` | Open terminal UI |

## Sessions

Use `--session` to isolate process groups (e.g. per-project):

```bash
agent-procs --session projectA run "make serve" --name app
agent-procs --session projectB run "make serve" --name app
agent-procs --session projectA status   # only shows projectA's processes
```

## Architecture

The CLI communicates with a per-session background daemon over a Unix domain socket. The daemon manages process lifecycles, captures stdout/stderr to log files, and handles wait conditions. The daemon auto-starts on first use and exits when all processes are stopped.

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error (timeout, connection failure, unexpected response) |
| 2 | No logs found for target process |
