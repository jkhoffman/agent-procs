---
name: agent-procs
description: "Manage concurrent background processes (dev servers, databases, builds, watchers) with agent-procs. Use this skill whenever you need to: run a process in the background, start multiple services, wait for a service to be ready, check logs of a running process, orchestrate a dev stack, write or modify an agent-procs.yaml config file, or any time a task involves long-running processes that need monitoring. If you're about to use nohup, &, or background a process manually, use agent-procs instead. Also use when the user mentions agent-procs directly, asks about process management, or has an agent-procs.yaml in their project."
---

# agent-procs — Concurrent Process Runner

`agent-procs` runs processes in a background daemon that persists across CLI invocations. It handles spawning, monitoring, log capture, readiness detection, and cleanup — so you don't have to cobble together `nohup`, `&`, and `tail -f`.

## When to use agent-procs vs plain Bash

**Use agent-procs when:**
- You need a process to keep running while you do other work (dev servers, file watchers, databases)
- You need to wait for a service to become ready before proceeding
- You're running 2+ long-lived processes concurrently
- You need to check logs of a background process later
- The project has an `agent-procs.yaml` config file

**Use plain Bash when:**
- You just need to run a command and read its output (e.g., `ls`, `cargo build`, `npm test`)
- The command is short-lived and you need the result immediately
- It's a single one-off command with no monitoring needs

## Quick reference

```bash
# Start processes
agent-procs run "npm run dev" --name server
agent-procs run "cargo watch -x test" --name tests

# Wait for readiness
agent-procs wait server --until "Listening on" --timeout 30

# Check status
agent-procs status                    # table view
agent-procs status --json             # machine-readable

# View logs
agent-procs logs server --tail 50     # last 50 lines
agent-procs logs server --stderr      # stderr only
agent-procs logs --all --tail 30      # all processes interleaved
agent-procs logs server --follow --timeout 10   # stream live for 10s

# Stop
agent-procs stop server               # one process
agent-procs stop-all                   # everything in session

# Config-driven
agent-procs up                         # start all from agent-procs.yaml
agent-procs down                       # stop all + shut down daemon
```

## Config-based startup (preferred for multi-service projects)

If a project needs multiple services, write an `agent-procs.yaml` in the project root. This is better than ad-hoc `run` commands because it captures the full topology — commands, working directories, environment, readiness checks, and dependency order.

### Config file format

```yaml
session: myproject          # optional — isolates this project's processes

processes:
  db:
    cmd: docker compose up postgres
    ready: "ready to accept connections"

  api:
    cmd: ./start-api-server
    cwd: ./backend          # relative to config file location
    env:
      DATABASE_URL: postgres://localhost:5432/mydb
      LOG_LEVEL: debug
    ready: "Listening on :8080"
    depends_on: [db]        # db must be ready before api starts

  frontend:
    cmd: npm run dev
    cwd: ./frontend
    ready: "Local:"
    depends_on: [api]
```

### Field reference

| Field | Required | Description |
|-------|----------|-------------|
| `cmd` | yes | Shell command to execute |
| `cwd` | no | Working directory (relative to config file) |
| `env` | no | Environment variables (key: value map) |
| `ready` | no | Stdout pattern that signals the process is ready |
| `depends_on` | no | List of process names that must be ready first |

Top-level `session` is optional and overridden by the `--session` CLI flag.

### Using the config

```bash
agent-procs up                    # start all, respects dependency order
agent-procs up --only db,api      # start only specific processes
agent-procs status                # verify everything is running
agent-procs down                  # stop all + shut down daemon
```

`up` starts processes in dependency order — independent processes launch concurrently, and each level waits for `ready` patterns (30s timeout) before starting the next.

## Waiting for readiness

Always wait for a service to be ready before using it. Always set a timeout.

```bash
# Wait for a pattern in stdout (literal substring match)
agent-procs wait server --until "Listening on" --timeout 30

# Regex matching
agent-procs wait server --until "listening on port \d+" --regex --timeout 30

# Wait for a process to exit (e.g., a build or migration)
agent-procs wait build --exit --timeout 120
```

Exit code 0 = condition met. Exit code 1 = timeout or error.

## Checking logs and status

```bash
# Status table
agent-procs status
# NAME        PID    STATE    EXIT  UPTIME
# server      12345  running  -     2m30s     <- healthy
# tests       12348  exited   0     -         <- completed OK
# build       12350  exited   1     -         <- FAILED, check logs

# Structured output for parsing
agent-procs status --json

# Logs
agent-procs logs server --tail 50          # last 50 lines of stdout
agent-procs logs server --stderr           # stderr
agent-procs logs --all --tail 30           # all processes interleaved

# Live streaming
agent-procs logs server --follow --timeout 10     # stream for 10 seconds
agent-procs logs server --follow --lines 20       # stream until 20 lines
```

If a process shows `exited` with non-zero exit code, check its logs and stderr.

## Session isolation

Sessions keep process groups separate. Useful when working on multiple projects.

```bash
agent-procs --session frontend run "npm run dev" --name web
agent-procs --session backend run "cargo run" --name api

agent-procs --session frontend status    # only frontend processes
agent-procs --session backend status     # only backend processes
```

If the config file has a `session:` field, `up`/`down` use it automatically.

```bash
agent-procs session list     # show active sessions
agent-procs session clean    # remove stale sessions (dead daemons)
```

## Monitoring with the TUI

```bash
agent-procs ui                           # current session
agent-procs ui --session myproject       # specific session
```

Keybindings: `j/k` or arrows to select process, `e` cycle stdout/stderr/both, `Space` pause, `u/d` or `PgUp/PgDn` scroll (auto-pauses), `g/G` top/bottom, `/` filter output, `Esc` clear filter, `r` restart, `x` stop, `X` stop-all, `q` quit, `Q` quit + stop all.

## Common recipes

**Full dev stack:**
```bash
agent-procs up                              # start from config
agent-procs status                          # verify all running
# ... do work ...
agent-procs down                            # clean up when done
```

**Background tests while editing:**
```bash
agent-procs run "cargo watch -x test" --name tests
# ... edit code ...
agent-procs logs tests --tail 20            # check results
agent-procs stop tests                      # done
```

**Start server, wait, then run integration tests:**
```bash
agent-procs run "npm start" --name server
agent-procs wait server --until "Listening" --timeout 30
npm run test:integration
agent-procs stop server
```

**Watch for errors in a build:**
```bash
agent-procs run "npm run build --watch" --name build
agent-procs wait build --until "error" --timeout 60
# exit 0 → error found, check logs
# exit 1 → timeout, no errors seen (good)
```

## Rules

1. **Always use `--timeout` on waits** — never wait without a timeout
2. **Check status after starting** — verify processes are actually running
3. **Clean up when done** — run `down` or `stop-all` to avoid orphan processes
4. **Use `--json` for parsing** — `agent-procs status --json` when you need structured data
5. **Check exit codes** — non-zero exit means something failed, check logs
6. **Prefer config files for repeatable setups** — if a project needs the same processes every time, write an `agent-procs.yaml`
