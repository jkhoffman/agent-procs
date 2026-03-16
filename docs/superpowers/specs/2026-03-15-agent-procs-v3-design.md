# agent-procs v3 Design Spec

TUI for human observation and basic control of agent-managed processes.

## Goals

1. Add `agent-procs ui` — a terminal UI for monitoring process output and intervening when needed
2. Read-only monitoring + basic control (stop, restart). No process launching from the TUI.
3. No daemon changes — the TUI is a new client using existing protocol capabilities.

## Architecture

The TUI is a new `ratatui`-based client that connects to the existing daemon socket. It uses two concurrent connections:

1. **Output stream** — `Request::Logs { follow: true, all: true }` for real-time output from all processes. Long-lived connection reading `Response::LogLine` messages.
2. **Command connection** — short-lived connections for `Request::Status` (periodic polling), `Request::Stop`, `Request::Restart`. Uses the existing `cli::request()` helper.

**One daemon-side change needed:** The follow stream handler defaults `timeout_secs: None` to 30 seconds. The TUI needs an indefinite stream. Change the daemon to treat `None` as "no timeout" (infinite) for follow streams. CLI `--follow` without `--timeout` will also benefit — it currently defaults to 30s, which will now become indefinite (the `--timeout` flag still works for explicit limits). This is a small change to `server.rs`.

**Output stream connection:** The TUI cannot reuse `cli::stream_responses()` (which blocks with a synchronous callback). Instead, the TUI uses `cli::connect()` directly, sends the `Request::Logs` manually, and reads `Response::LogLine` messages in its own async loop, feeding lines into an `mpsc` channel that the event loop consumes. If the connection drops (daemon restart), the TUI auto-reconnects.

**Event loop:** Two async tasks feed into shared `App` state:
- Daemon reader: receives output lines and status updates
- Input handler: reads terminal key events via `crossterm`, maps to state transitions

The render loop redraws on any state change.

## New Files

```
src/tui/
  mod.rs          # TUI entry point, terminal setup/teardown, main event loop
  app.rs          # App state: selected process, scroll position, output buffers, mode
  ui.rs           # ratatui rendering: layout, widgets, colors, status bar
  input.rs        # Keyboard event handling → app state transitions
```

**Dependencies to add:**
- `ratatui = "0.29"` (TUI framework)
- `crossterm = "0.28"` (terminal input/output backend)

## UI Layout

```
┌─ Processes ──────┬─ Output: webserver (stdout) ─────────────┐
│                  │                                           │
│ ● webserver      │  [14:23:01] Server starting              │
│   api            │  [14:23:02] Listening on :3000           │
│   db             │  [14:23:05] GET / 200 12ms               │
│ ✗ build (1)      │  [14:23:06] GET /api 200 8ms             │
│                  │  [14:23:07] GET /health 200 1ms          │
│                  │  [14:23:08] POST /users 201 45ms         │
│                  │                                           │
├──────────────────┴──────────────────────────────────────────┤
│ ↑↓ select  r restart  x stop  X stop-all  e stream         │
│ space pause  q quit  Q quit+stop     3 running, 1 exited   │
└─────────────────────────────────────────────────────────────┘
```

### Left Pane — Process List

- Status indicator: `●` running (green), `✗` exited non-zero (red), `✓` exited 0 (dim)
- Selected process highlighted with background color
- Shows exit code in parens for exited processes (e.g., `✗ build (1)`)
- Updated via periodic `Request::Status` polling (every 1-2 seconds)

### Right Pane — Output

- Shows buffered output for selected process
- Title bar shows process name + current stream mode: `(stdout)`, `(stderr)`, or `(all)`
- Auto-scrolls to bottom unless paused
- In "all" mode, stderr lines are colored (red/yellow) to distinguish from stdout
- Each process maintains its own ring buffer of last 10,000 lines in memory

### Bottom Bar

- Left: keybinding hints
- Right: summary count (N running, N exited)

## Keybindings

| Key | Action |
|-----|--------|
| `↑` / `k` | Select previous process |
| `↓` / `j` | Select next process |
| `r` | Restart selected process |
| `x` | Stop selected process |
| `X` | Stop all processes |
| `e` | Cycle stream mode: stdout → stderr → both → stdout |
| `Space` | Pause/resume auto-scroll |
| `q` | Quit TUI (processes keep running) |
| `Q` | Stop all processes and quit |

## App State

```rust
struct App {
    processes: Vec<ProcessInfo>,        // from Status polling
    selected: usize,                    // index into processes
    buffers: HashMap<String, OutputBuffer>,  // per-process output ring buffers
    stream_mode: StreamMode,            // Stdout, Stderr, Both
    paused: bool,                       // auto-scroll paused
    scroll_offsets: HashMap<String, usize>,  // per-process scroll offset when paused
    running: bool,                     // false when user quits
}

enum StreamMode { Stdout, Stderr, Both }

struct OutputBuffer {
    stdout: VecDeque<String>,          // ring buffer, max 10k lines
    stderr: VecDeque<String>,          // ring buffer, max 10k lines
}
```

## Key Behaviors

- **Quit vs Quit+Stop:** `q` disconnects the TUI but processes keep running (the daemon is independent). `Q` sends `StopAll` then quits.
- **Pause:** Freezes scroll position. Output keeps buffering in the ring buffer. Unpause (`Space` again) jumps to the latest output.
- **Process selection:** Switching processes shows that process's buffered output immediately (no re-fetch needed — it's all in memory from the follow stream).
- **Stream toggle:** `e` cycles stdout → stderr → both. In "both" mode, lines are interleaved chronologically (they arrive interleaved from the daemon). Stderr lines are visually distinct.
- **New process appears:** If the agent starts a new process while the TUI is running, it appears in the process list on the next status poll. Its output starts buffering immediately (the follow stream includes all processes).
- **Process exits:** Status poll updates the indicator. Output remains in the buffer for inspection.

## Terminal Setup/Teardown

- Enter raw mode and alternate screen on startup
- Restore terminal on exit (even on panic — use a panic hook)
- Handle SIGINT/SIGTERM gracefully (restore terminal, then exit)

## Files Changed

- `src/daemon/server.rs` (modify): Change follow stream to treat `timeout_secs: None` as infinite (no timeout). Use `tokio::time::timeout` only when `Some(secs)` is provided.
- `src/tui/mod.rs` (create): Terminal setup/teardown, main event loop, daemon connection management, auto-reconnect
- `src/tui/app.rs` (create): App state, output buffer management, state transitions
- `src/tui/ui.rs` (create): Rendering with ratatui (layout, process list widget, output widget, status bar)
- `src/tui/input.rs` (create): Key event → action mapping
- `src/lib.rs` (modify): Add `pub mod tui;`
- `src/main.rs` (modify): Add `Commands::Ui` variant (no arguments — uses global `--session` flag) and wire to TUI entry point
- `Cargo.toml` (modify): Add `ratatui` and `crossterm` dependencies

## Testing

TUI testing is primarily manual — automated testing of terminal UIs is fragile and low-value. However:

- **Unit test `App` state transitions:** Test that key actions produce correct state changes (select next, toggle stream, pause, etc.) without rendering.
- **Unit test `OutputBuffer`:** Test ring buffer behavior, line limits, stream filtering.
- **Integration smoke test:** Start a process, launch `agent-procs ui` briefly (with a timeout), verify it exits cleanly. This just tests that the TUI starts and stops without crashing.

## Known Limitations

- **Broadcast channel lag:** The daemon's broadcast channel has a capacity of 1024. High-output processes may cause the TUI's receiver to lag, silently dropping lines. This is acceptable for v3 — most processes don't produce output that fast.
- **No scroll-back while unpaused:** Output that scrolls past the visible area while unpaused cannot be reviewed until v4 adds scroll navigation. Pause early if you need to read output.

## Explicitly Out of Scope (v3)

- Log search / text filtering (v4)
- Scroll-back through history with arrow keys (v4 — requires scroll navigation beyond just pause)
- Launching new processes from the TUI
- Mouse support
- Configurable keybindings
- Configurable layout or themes
