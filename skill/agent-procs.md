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
# webserver   12345  running  -     2m30s    <- healthy
# tests       12348  exited   0     -        <- completed successfully
# build       12350  exited   1     -        <- FAILED -- check logs
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
