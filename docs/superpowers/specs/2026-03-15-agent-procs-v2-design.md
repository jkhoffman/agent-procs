# agent-procs v2 Design Spec

Robustness fixes and `--follow` streaming for agent-procs.

## Goals

1. Fix correctness issues discovered during v1 implementation
2. Add real-time log streaming (`--follow`) for efficient agent output consumption
3. No new commands or architectural changes — this is a refinement release

## Change 1: Process Groups

### Problem

`agent-procs stop` sends SIGTERM to the `sh` process that wraps the user's command, but child processes (the actual server, database, etc.) may survive as orphans.

### Solution

Spawn each child in its own process group. Signal the group on stop.

- Call `setpgid(0, 0)` in an `unsafe` `CommandExt::pre_exec` closure before spawning the child (standard Rust pattern for pre-exec hooks)
- The child PID equals its PGID (since it becomes the process group leader). The existing `child.id()` value serves as both PID and PGID.
- `stop_process()` uses `killpg(pgid, SIGTERM)` instead of `kill(pid, SIGTERM)`
- SIGKILL escalation also uses `killpg(pgid, SIGKILL)` after the 10s timeout

### Files Changed

- `src/daemon/process_manager.rs`: Add `pre_exec` hook with `setpgid`, change stop to use `killpg`

## Change 2: Socket Path Fix

### Problem

macOS limits Unix socket paths to 103 bytes. With XDG_RUNTIME_DIR + nested session directories, real-world paths easily exceed this, causing silent connection failures.

### Solution

Move sockets and PID files to a short fixed path. Logs and state stay under XDG.

**New layout:**
```
/tmp/agent-procs-<uid>/<session>.sock       # socket
/tmp/agent-procs-<uid>/<session>.pid        # PID file

$XDG_STATE_HOME/agent-procs/sessions/<id>/  # logs, state.json (unchanged)
```

The flat structure replaces the nested `$XDG_RUNTIME_DIR/agent-procs/sessions/<id>/` directories. Session names are used directly as filenames with `.sock` and `.pid` extensions.

The socket directory should be created with mode `0700` to prevent other users from connecting.

### Session cleanup

`session_cmd.rs` must handle both locations:
- Scan `/tmp/agent-procs-<uid>/` for `.pid` files to find sessions and check liveness
- When cleaning a stale session, remove both the socket/PID files AND the corresponding `$XDG_STATE_HOME` state directory (logs, state.json)

Split `sessions_base_dir()` into `socket_base_dir()` (returns `/tmp/agent-procs-<uid>/`) for socket/PID operations.

### Files Changed

- `src/paths.rs`: Rewrite `socket_path()` and `pid_path()` to use `/tmp/agent-procs-<uid>/`. Remove `runtime_dir()` and `runtime_base()`. Rename `sessions_base_dir()` to `socket_base_dir()`.
- `src/daemon/spawn.rs`: Create `/tmp/agent-procs-<uid>/` (mode 0700) instead of old runtime dir. State dir creation is unchanged.
- `src/cli/session_cmd.rs`: Scan `socket_base_dir()` for `<session>.pid` files. On clean, remove both socket/PID files and the XDG state directory.
- `src/session.rs`: `is_daemon_alive()` is unchanged (it takes a path, works with any PID file location)
- `tests/`: Session names no longer constrained to ~10 chars

## Change 3: Protocol-Level Env Vars

### Problem

Environment variables in `agent-procs.yaml` are passed via shell `export` prefix (`export KEY=VALUE && cmd`). This breaks with values containing shell metacharacters (`&&`, backticks, `$`, etc.).

### Solution

Add `env` field to `Request::Run` and use `Command::envs()` to set them directly.

**Protocol change:**
```rust
Request::Run {
    command: String,
    name: Option<String>,
    cwd: Option<String>,
    env: Option<HashMap<String, String>>,  // NEW
}
```

The daemon calls `cmd.envs(env.unwrap_or_default())` before spawning. This handles all special characters correctly with no escaping needed.

The `env` field uses `#[serde(default)]` for forward compatibility — a v1 daemon receiving a v2 request without the field will deserialize it as `None`. In practice the daemon is auto-spawned from the same binary, so version mismatch is unlikely.

### Files Changed

- `src/protocol.rs`: Add `env: Option<HashMap<String, String>>` to `Request::Run`
- `src/daemon/process_manager.rs`: Call `cmd.envs()` if env is present
- `src/cli/up.rs`: Pass `def.env` through the protocol. Remove `shell_escape()` function and export prefix logic.
- `src/cli/run.rs`: Pass `env: None`
- `tests/test_protocol.rs`: Update roundtrip test for new field

## Change 4: `--follow` Streaming

### Problem

Agents can only read logs via `--tail` (snapshot). There's no way to stream output in real-time, forcing agents to poll in a loop.

### Solution

Wire the daemon's existing `broadcast::Sender<OutputLine>` channel to the CLI through the socket protocol.

**CLI interface:**
```bash
agent-procs logs server --follow                          # stream, default 30s timeout
agent-procs logs server --follow --timeout 60             # custom timeout
agent-procs logs server --follow --lines 50               # stop after 50 lines
agent-procs logs server --follow --lines 50 --timeout 60  # whichever first
agent-procs logs --all --follow --timeout 30              # all processes
```

**Protocol flow:**

1. CLI sends `Request::Logs { follow: true, target, timeout_secs, lines, ... }` to daemon
2. Daemon subscribes to the broadcast channel, filters by target (or all if `target` is None and `all` is true)
3. For each matching line, daemon sends `Response::LogLine { process, stream, line }`
4. When timeout or line limit is reached, daemon sends `Response::LogEnd`
5. CLI prints each `LogLine` as it arrives, exits on `LogEnd`

**Protocol changes:**
```rust
Request::Logs {
    target: Option<String>,
    tail: usize,
    follow: bool,
    stderr: bool,
    all: bool,
    timeout_secs: Option<u64>,
    lines: Option<usize>,          // NEW: max lines for --follow
}
```

`Response::LogLine` and `Response::LogEnd` already exist in the v1 protocol (defined but unused). No new response types are needed.

**Non-follow mode is unchanged** — the CLI still reads log files directly from disk. Only `--follow` goes through the daemon.

### Files Changed

- `src/protocol.rs`: Add `lines: Option<usize>` to `Request::Logs`
- `src/main.rs`: Add `--lines` flag to `Logs` command
- `src/daemon/server.rs`: Handle `Request::Logs` with `follow: true` — subscribe to broadcast channel, stream filtered lines, respect timeout and line limit
- `src/cli/logs.rs`: When `--follow`, send request to daemon and read streaming responses. Remove "not yet implemented" warning.
- `src/cli/mod.rs`: Add a streaming reader helper — the current `cli::request()` reads exactly one response and returns. `--follow` needs a loop that reads multiple `Response::LogLine` messages until `Response::LogEnd`. Add `cli::stream_responses()` or similar.

### Edge Cases

- `--follow --all`: Streams from all processes, each `LogLine` includes process name for prefixing
- `--follow` on an exited process: Daemon drains any remaining buffered lines from the broadcast channel, then sends `LogEnd`. This ensures final output lines from a just-exited process are not lost.
- `--follow` with `--tail`: First emit the last N lines from disk, then stream live. This is a nice-to-have — v2 can require choosing one or the other.

## Explicitly Out of Scope (v2)

- TUI (v3)
- `--env KEY=VALUE` flag for ad-hoc `run` command (the protocol supports it now, CLI flag can come later)
- `--follow` combined with `--tail` (emit historical + live)
- Restart policies
- stdin forwarding
- Windows support
