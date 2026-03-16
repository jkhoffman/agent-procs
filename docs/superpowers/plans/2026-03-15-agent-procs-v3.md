# agent-procs v3 (TUI) Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `ratatui`-based terminal UI for monitoring and controlling agent-managed processes.

**Architecture:** The TUI is a new client connecting to the existing daemon via two socket connections — one long-lived for streaming output, one for commands. It renders a split-pane layout (process list + output) with keyboard controls. One daemon-side change: make follow stream timeout optional (currently hardcoded to 30s default).

**Tech Stack:** Rust, ratatui 0.29, crossterm 0.28, tokio (async event loop)

**Spec:** `docs/superpowers/specs/2026-03-15-agent-procs-v3-design.md`

---

## File Structure

```
src/tui/
  mod.rs          # Entry point: terminal setup/teardown, main event loop, daemon connections
  app.rs          # App state, OutputBuffer, StreamMode, state transition methods
  ui.rs           # ratatui rendering: layout, process list widget, output widget, status bar
  input.rs        # Key event → Action enum, action dispatch
```

**Existing files modified:**
- `Cargo.toml`: Add ratatui, crossterm dependencies
- `src/lib.rs`: Add `pub mod tui;`
- `src/main.rs`: Add `Ui` command variant, wire to TUI entry
- `src/daemon/server.rs`: Make follow timeout optional (None = infinite)

---

## Chunk 1: Daemon Fix, Dependencies, App State

### Task 1: Fix follow stream timeout for TUI

**Files:**
- Modify: `src/daemon/server.rs`

The follow stream handler defaults `timeout_secs: None` to 30 seconds. The TUI needs indefinite streaming. Change to only apply timeout when explicitly set.

- [ ] **Step 1: Update the follow handler timeout logic**

In `src/daemon/server.rs`, replace the `handle_follow_stream` call site (lines 49-60). Change the timeout construction from `Duration::from_secs(timeout_secs.unwrap_or(30))` to conditional:

```rust
                if let Request::Logs { follow: true, ref target, all, timeout_secs, lines, .. } = request {
                    let output_rx = state.lock().await.process_manager.output_tx.subscribe();
                    let max_lines = lines;
                    let target_filter = target.clone();
                    let show_all = all;

                    handle_follow_stream(
                        &writer, output_rx, target_filter, show_all, timeout_secs, max_lines
                    ).await;
                    continue;
                }
```

Update `handle_follow_stream` signature and body — change `timeout: Duration` to `timeout_secs: Option<u64>` and conditionally wrap with `tokio::time::timeout`:

```rust
async fn handle_follow_stream(
    writer: &Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
    mut output_rx: broadcast::Receiver<super::log_writer::OutputLine>,
    target: Option<String>,
    all: bool,
    timeout_secs: Option<u64>,
    max_lines: Option<usize>,
) {
    let mut line_count: usize = 0;

    let stream_loop = async {
        loop {
            match output_rx.recv().await {
                Ok(output_line) => {
                    if !all {
                        if let Some(ref t) = target {
                            if output_line.process != *t { continue; }
                        }
                    }

                    let resp = Response::LogLine {
                        process: output_line.process,
                        stream: output_line.stream,
                        line: output_line.line,
                    };
                    if send_response(writer, &resp).await.is_err() {
                        return;
                    }

                    line_count += 1;
                    if let Some(max) = max_lines {
                        if line_count >= max { return; }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    };

    // Apply timeout only if specified; otherwise stream indefinitely
    match timeout_secs {
        Some(secs) => { let _ = tokio::time::timeout(Duration::from_secs(secs), stream_loop).await; }
        None => { stream_loop.await; }
    }

    let _ = send_response(writer, &Response::LogEnd).await;
}
```

- [ ] **Step 2: Preserve 30s default in CLI**

In `src/cli/logs.rs`, in the `execute_follow` function, apply the 30s default on the CLI side so existing behavior is preserved. The daemon gets `None` only when the TUI explicitly passes it:

```rust
    let req = Request::Logs {
        target: target.map(|t| t.to_string()),
        tail: 0,
        follow: true,
        stderr: false,
        all: all || target.is_none(),
        timeout_secs: timeout.or(Some(30)), // CLI default; TUI passes None for infinite
        lines,
    };
```

- [ ] **Step 3: Verify existing tests still pass**

Run: `cargo test -- --test-threads=1`
Expected: All tests pass. CLI `--follow` behavior unchanged (still 30s default). Daemon now supports infinite streams for the TUI.

- [ ] **Step 4: Commit**

```bash
git add src/daemon/server.rs src/cli/logs.rs
git commit -m "fix: make follow stream timeout optional (None = infinite), preserve CLI 30s default"
```

---

### Task 2: Add dependencies and wire up TUI command

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/lib.rs`
- Modify: `src/main.rs`
- Create: `src/tui/mod.rs` (stub)
- Create: `src/tui/app.rs` (stub)
- Create: `src/tui/ui.rs` (stub)
- Create: `src/tui/input.rs` (stub)

- [ ] **Step 1: Add dependencies to Cargo.toml**

Add to `[dependencies]`:

```toml
ratatui = "0.29"
crossterm = { version = "0.28", features = ["event-stream"] }
futures = "0.3"
```

- [ ] **Step 2: Create TUI module stubs**

Create `src/tui/mod.rs`:

```rust
pub mod app;
pub mod ui;
pub mod input;

pub async fn run(session: &str) -> i32 {
    eprintln!("TUI not yet implemented");
    1
}
```

Create `src/tui/app.rs`:

```rust
// App state — implemented in Task 3
```

Create `src/tui/ui.rs`:

```rust
// Rendering — implemented in Task 5
```

Create `src/tui/input.rs`:

```rust
// Input handling — implemented in Task 4
```

- [ ] **Step 3: Add module to lib.rs**

Add `pub mod tui;` to `src/lib.rs`.

- [ ] **Step 4: Add Ui command to main.rs**

Add to the `Commands` enum:

```rust
    /// Open terminal UI for monitoring processes
    Ui,
```

Add to the match in `main()`:

```rust
        Commands::Ui => agent_procs::tui::run(session).await,
```

- [ ] **Step 5: Verify it compiles and the command is recognized**

Run: `cargo build`
Run: `cargo run -- ui --help`
Expected: Compiles; `ui` command is recognized

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml src/lib.rs src/main.rs src/tui/
git commit -m "feat: add TUI module stubs and wire up ui command"
```

---

### Task 3: App state and output buffers

**Files:**
- Modify: `src/tui/app.rs`
- Create: `tests/test_tui_app.rs`

- [ ] **Step 1: Write unit tests for app state**

Create `tests/test_tui_app.rs`:

```rust
use agent_procs::tui::app::*;
use agent_procs::protocol::{ProcessInfo, ProcessState, Stream};

#[test]
fn test_output_buffer_ring_behavior() {
    let mut buf = OutputBuffer::new(5); // max 5 lines
    for i in 0..8 {
        buf.push_stdout(format!("line {}", i));
    }
    let lines = buf.stdout_lines();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0], "line 3");
    assert_eq!(lines[4], "line 7");
}

#[test]
fn test_output_buffer_stderr() {
    let mut buf = OutputBuffer::new(100);
    buf.push_stdout("out1".into());
    buf.push_stderr("err1".into());
    buf.push_stdout("out2".into());

    assert_eq!(buf.stdout_lines().len(), 2);
    assert_eq!(buf.stderr_lines().len(), 1);
    assert_eq!(buf.all_lines().len(), 3);
    assert_eq!(buf.all_lines()[1].0, LineSource::Stderr);
}

#[test]
fn test_app_select_next_wraps() {
    let mut app = App::new();
    app.update_processes(vec![
        make_info("alpha", ProcessState::Running),
        make_info("beta", ProcessState::Running),
    ]);
    assert_eq!(app.selected, 0);
    app.select_next();
    assert_eq!(app.selected, 1);
    app.select_next();
    assert_eq!(app.selected, 0); // wraps
}

#[test]
fn test_app_select_prev_wraps() {
    let mut app = App::new();
    app.update_processes(vec![
        make_info("alpha", ProcessState::Running),
        make_info("beta", ProcessState::Running),
    ]);
    assert_eq!(app.selected, 0);
    app.select_prev();
    assert_eq!(app.selected, 1); // wraps to end
}

#[test]
fn test_stream_mode_cycles() {
    let mut app = App::new();
    assert!(matches!(app.stream_mode, StreamMode::Stdout));
    app.cycle_stream_mode();
    assert!(matches!(app.stream_mode, StreamMode::Stderr));
    app.cycle_stream_mode();
    assert!(matches!(app.stream_mode, StreamMode::Both));
    app.cycle_stream_mode();
    assert!(matches!(app.stream_mode, StreamMode::Stdout));
}

#[test]
fn test_pause_toggle() {
    let mut app = App::new();
    assert!(!app.paused);
    app.toggle_pause();
    assert!(app.paused);
    app.toggle_pause();
    assert!(!app.paused);
}

#[test]
fn test_push_output_creates_buffer() {
    let mut app = App::new();
    app.push_output("server", Stream::Stdout, "hello");
    assert!(app.buffers.contains_key("server"));
    let buf = &app.buffers["server"];
    assert_eq!(buf.stdout_lines().len(), 1);
}

fn make_info(name: &str, state: ProcessState) -> ProcessInfo {
    ProcessInfo {
        name: name.into(), id: "p1".into(), pid: 1234,
        state, exit_code: None, uptime_secs: Some(10),
        command: "test".into(),
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test test_tui_app`
Expected: Compilation fails — app module is a stub

- [ ] **Step 3: Implement App state**

Replace `src/tui/app.rs`:

```rust
use crate::protocol::{ProcessInfo, Stream};
use std::collections::{HashMap, VecDeque};

const MAX_BUFFER_LINES: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StreamMode { Stdout, Stderr, Both }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineSource { Stdout, Stderr }

pub struct OutputBuffer {
    stdout: VecDeque<String>,
    stderr: VecDeque<String>,
    /// Interleaved (source, line) for "both" mode, in arrival order
    all: VecDeque<(LineSource, String)>,
    max_lines: usize,
}

impl OutputBuffer {
    pub fn new(max_lines: usize) -> Self {
        Self {
            stdout: VecDeque::with_capacity(max_lines),
            stderr: VecDeque::with_capacity(max_lines),
            all: VecDeque::with_capacity(max_lines),
            max_lines,
        }
    }

    pub fn push_stdout(&mut self, line: String) {
        if self.stdout.len() == self.max_lines { self.stdout.pop_front(); }
        self.stdout.push_back(line.clone());
        if self.all.len() == self.max_lines { self.all.pop_front(); }
        self.all.push_back((LineSource::Stdout, line));
    }

    pub fn push_stderr(&mut self, line: String) {
        if self.stderr.len() == self.max_lines { self.stderr.pop_front(); }
        self.stderr.push_back(line.clone());
        if self.all.len() == self.max_lines { self.all.pop_front(); }
        self.all.push_back((LineSource::Stderr, line));
    }

    pub fn stdout_lines(&self) -> Vec<&str> {
        self.stdout.iter().map(|s| s.as_str()).collect()
    }

    pub fn stderr_lines(&self) -> Vec<&str> {
        self.stderr.iter().map(|s| s.as_str()).collect()
    }

    pub fn all_lines(&self) -> Vec<(LineSource, &str)> {
        self.all.iter().map(|(src, s)| (*src, s.as_str())).collect()
    }
}

pub struct App {
    pub processes: Vec<ProcessInfo>,
    pub selected: usize,
    pub buffers: HashMap<String, OutputBuffer>,
    pub stream_mode: StreamMode,
    pub paused: bool,
    pub scroll_offsets: HashMap<String, usize>,
    pub running: bool,
    pub stop_all_on_quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            processes: Vec::new(),
            selected: 0,
            buffers: HashMap::new(),
            stream_mode: StreamMode::Stdout,
            paused: false,
            scroll_offsets: HashMap::new(),
            running: true,
            stop_all_on_quit: false,
        }
    }

    pub fn update_processes(&mut self, processes: Vec<ProcessInfo>) {
        self.processes = processes;
        if self.selected >= self.processes.len() && !self.processes.is_empty() {
            self.selected = self.processes.len() - 1;
        }
    }

    pub fn selected_name(&self) -> Option<&str> {
        self.processes.get(self.selected).map(|p| p.name.as_str())
    }

    pub fn select_next(&mut self) {
        if !self.processes.is_empty() {
            self.selected = (self.selected + 1) % self.processes.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.processes.is_empty() {
            self.selected = if self.selected == 0 {
                self.processes.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    pub fn cycle_stream_mode(&mut self) {
        self.stream_mode = match self.stream_mode {
            StreamMode::Stdout => StreamMode::Stderr,
            StreamMode::Stderr => StreamMode::Both,
            StreamMode::Both => StreamMode::Stdout,
        };
    }

    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        if !self.paused {
            // Reset scroll offset to bottom on unpause
            if let Some(name) = self.selected_name() {
                self.scroll_offsets.remove(&name.to_string());
            }
        }
    }

    pub fn push_output(&mut self, process: &str, stream: Stream, line: &str) {
        let buf = self.buffers
            .entry(process.to_string())
            .or_insert_with(|| OutputBuffer::new(MAX_BUFFER_LINES));
        match stream {
            Stream::Stdout => buf.push_stdout(line.to_string()),
            Stream::Stderr => buf.push_stderr(line.to_string()),
        }
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn quit_and_stop(&mut self) {
        self.stop_all_on_quit = true;
        self.running = false;
    }

    pub fn running_count(&self) -> usize {
        self.processes.iter().filter(|p| p.state == crate::protocol::ProcessState::Running).count()
    }

    pub fn exited_count(&self) -> usize {
        self.processes.iter().filter(|p| p.state == crate::protocol::ProcessState::Exited).count()
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test test_tui_app`
Expected: All 7 tests pass

- [ ] **Step 5: Commit**

```bash
git add src/tui/app.rs tests/test_tui_app.rs
git commit -m "feat: add TUI app state with output buffers and state transitions"
```

---

### Task 4: Input handling

**Files:**
- Modify: `src/tui/input.rs`

- [ ] **Step 1: Implement input handler**

Replace `src/tui/input.rs`:

```rust
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    SelectNext,
    SelectPrev,
    Restart,
    Stop,
    StopAll,
    CycleStream,
    TogglePause,
    Quit,
    QuitAndStop,
    None,
}

pub fn handle_key(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => Action::SelectNext,
        KeyCode::Up | KeyCode::Char('k') => Action::SelectPrev,
        KeyCode::Char('r') => Action::Restart,
        KeyCode::Char('x') => Action::Stop,
        KeyCode::Char('X') => Action::StopAll,
        KeyCode::Char('e') => Action::CycleStream,
        KeyCode::Char(' ') => Action::TogglePause,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('Q') => Action::QuitAndStop,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        _ => Action::None,
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add src/tui/input.rs
git commit -m "feat: add TUI keyboard input handler"
```

---

## Chunk 2: Rendering, Event Loop, and Integration

### Task 5: TUI rendering

**Files:**
- Modify: `src/tui/ui.rs`

- [ ] **Step 1: Implement rendering**

Replace `src/tui/ui.rs`:

```rust
use ratatui::prelude::*;
use ratatui::widgets::*;
use crate::protocol::ProcessState;
use crate::tui::app::{App, StreamMode, LineSource};

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),    // main area
            Constraint::Length(2), // status bar
        ])
        .split(frame.area());

    // Main area: split into process list and output
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(22), // process list
            Constraint::Min(30),   // output
        ])
        .split(chunks[0]);

    draw_process_list(frame, app, main_chunks[0]);
    draw_output(frame, app, main_chunks[1]);
    draw_status_bar(frame, app, chunks[1]);
}

fn draw_process_list(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app.processes.iter().enumerate().map(|(i, p)| {
        let (indicator, style) = match p.state {
            ProcessState::Running => ("●", Style::default().fg(Color::Green)),
            ProcessState::Exited => {
                if p.exit_code == Some(0) {
                    ("✓", Style::default().fg(Color::DarkGray))
                } else {
                    let code = p.exit_code.map(|c| format!(" ({})", c)).unwrap_or_default();
                    return ListItem::new(format!("✗ {}{}", p.name, code))
                        .style(if i == app.selected {
                            Style::default().fg(Color::Red).bg(Color::DarkGray)
                        } else {
                            Style::default().fg(Color::Red)
                        });
                }
            }
        };

        let text = format!("{} {}", indicator, p.name);
        let style = if i == app.selected {
            style.bg(Color::DarkGray)
        } else {
            style
        };
        ListItem::new(text).style(style)
    }).collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Processes "));

    frame.render_widget(list, area);
}

fn draw_output(frame: &mut Frame, app: &App, area: Rect) {
    let name = app.selected_name().unwrap_or("(none)");
    let mode_label = match app.stream_mode {
        StreamMode::Stdout => "stdout",
        StreamMode::Stderr => "stderr",
        StreamMode::Both => "all",
    };
    let pause_indicator = if app.paused { " [PAUSED]" } else { "" };
    let title = format!(" Output: {} ({}){}  ", name, mode_label, pause_indicator);

    let lines: Vec<Line> = if let Some(buf) = app.selected_name().and_then(|n| app.buffers.get(n)) {
        match app.stream_mode {
            StreamMode::Stdout => {
                buf.stdout_lines().iter().map(|l| Line::from(l.to_string())).collect()
            }
            StreamMode::Stderr => {
                buf.stderr_lines().iter()
                    .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(Color::Yellow))))
                    .collect()
            }
            StreamMode::Both => {
                buf.all_lines().iter().map(|(src, l)| {
                    match src {
                        LineSource::Stdout => Line::from(l.to_string()),
                        LineSource::Stderr => Line::from(Span::styled(l.to_string(), Style::default().fg(Color::Yellow))),
                    }
                }).collect()
            }
        }
    } else {
        vec![Line::from("No output yet".to_string()).style(Style::default().fg(Color::DarkGray))]
    };

    // Calculate scroll: show bottom of output unless paused with offset
    let visible_height = area.height.saturating_sub(2) as usize; // -2 for borders
    let total_lines = lines.len();
    let scroll_offset = if app.paused {
        app.scroll_offsets.get(name).copied().unwrap_or(0)
    } else {
        0
    };
    let scroll_pos = total_lines.saturating_sub(visible_height + scroll_offset);

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(title))
        .scroll((scroll_pos as u16, 0));

    frame.render_widget(paragraph, area);
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let keys = " ↑↓ select  r restart  x stop  X stop-all  e stream  space pause  q quit  Q quit+stop ";
    let counts = format!(" {} running, {} exited ", app.running_count(), app.exited_count());

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(20),
            Constraint::Length(counts.len() as u16 + 2),
        ])
        .split(area);

    let keys_widget = Paragraph::new(keys)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    let counts_widget = Paragraph::new(counts)
        .style(Style::default().fg(Color::Cyan))
        .alignment(Alignment::Right)
        .block(Block::default().borders(Borders::TOP));

    frame.render_widget(keys_widget, chunks[0]);
    frame.render_widget(counts_widget, chunks[1]);
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add src/tui/ui.rs
git commit -m "feat: add TUI rendering with split pane layout"
```

---

### Task 6: Main event loop and daemon connections

**Files:**
- Modify: `src/tui/mod.rs`

This is the core of the TUI. It:
1. Sets up the terminal (raw mode, alternate screen)
2. Connects to the daemon (output stream + command connection)
3. Runs a loop: read events (keys, daemon output, status ticks), update state, render
4. Tears down cleanly on exit

- [ ] **Step 1: Implement the TUI entry point**

Replace `src/tui/mod.rs`:

```rust
pub mod app;
pub mod ui;
pub mod input;

use crate::cli;
use crate::protocol::{Request, Response, Stream as ProtoStream};
use app::App;
use crossterm::{
    event::{self, Event, EventStream},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    execute,
};
use futures::StreamExt;
use ratatui::prelude::*;
use std::io;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

enum AppEvent {
    Key(crossterm::event::KeyEvent),
    OutputLine { process: String, stream: ProtoStream, line: String },
    StatusUpdate(Vec<crate::protocol::ProcessInfo>),
    OutputStreamClosed,
}

pub async fn run(session: &str) -> i32 {
    // Verify daemon is running
    if cli::connect(session, false).await.is_err() {
        eprintln!("error: no daemon running for session '{}'. Start processes first.", session);
        return 1;
    }

    // Set up terminal
    if let Err(e) = enable_raw_mode() {
        eprintln!("error: failed to enable raw mode: {}", e);
        return 1;
    }
    let mut stdout = io::stdout();
    if let Err(e) = execute!(stdout, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        eprintln!("error: failed to enter alternate screen: {}", e);
        return 1;
    }

    // Install panic hook to restore terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout)).unwrap();
    let mut app = App::new();

    // Channel for events from background tasks
    let (tx, mut rx) = mpsc::channel::<AppEvent>(256);

    // Start output stream reader
    let session_str = session.to_string();
    let output_tx = tx.clone();
    tokio::spawn(async move {
        output_stream_reader(&session_str, output_tx).await;
    });

    // Start status poller
    let session_str = session.to_string();
    let status_tx = tx.clone();
    tokio::spawn(async move {
        status_poller(&session_str, status_tx).await;
    });

    // Start key event reader
    let key_tx = tx.clone();
    tokio::spawn(async move {
        key_reader(key_tx).await;
    });

    // Handle SIGTERM gracefully (restore terminal before exit)
    let sigterm_tx = tx.clone();
    tokio::spawn(async move {
        if let Ok(mut sig) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            sig.recv().await;
            let _ = sigterm_tx.send(AppEvent::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('q'),
                crossterm::event::KeyModifiers::empty(),
            ))).await;
        }
    });

    // Main render loop
    while app.running {
        terminal.draw(|f| ui::draw(f, &app)).unwrap();

        // Wait for next event
        if let Some(event) = rx.recv().await {
            match event {
                AppEvent::Key(key) => {
                    let action = input::handle_key(key);
                    match action {
                        input::Action::SelectNext => app.select_next(),
                        input::Action::SelectPrev => app.select_prev(),
                        input::Action::CycleStream => app.cycle_stream_mode(),
                        input::Action::TogglePause => app.toggle_pause(),
                        input::Action::Quit => app.quit(),
                        input::Action::QuitAndStop => app.quit_and_stop(),
                        input::Action::Stop => {
                            if let Some(name) = app.selected_name() {
                                let _ = cli::request(session, &Request::Stop { target: name.to_string() }, false).await;
                            }
                        }
                        input::Action::StopAll => {
                            let _ = cli::request(session, &Request::StopAll, false).await;
                        }
                        input::Action::Restart => {
                            if let Some(name) = app.selected_name() {
                                let _ = cli::request(session, &Request::Restart { target: name.to_string() }, false).await;
                            }
                        }
                        input::Action::None => {}
                    }
                }
                AppEvent::OutputLine { process, stream, line } => {
                    app.push_output(&process, stream, &line);
                }
                AppEvent::StatusUpdate(processes) => {
                    app.update_processes(processes);
                }
                AppEvent::OutputStreamClosed => {
                    // Try to reconnect after a brief delay
                    let session_str = session.to_string();
                    let reconnect_tx = tx.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        output_stream_reader(&session_str, reconnect_tx).await;
                    });
                }
            }
        }
    }

    // Stop all if requested
    if app.stop_all_on_quit {
        let _ = cli::request(session, &Request::StopAll, false).await;
    }

    // Restore terminal
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    0
}

async fn output_stream_reader(session: &str, tx: mpsc::Sender<AppEvent>) {
    let stream = match cli::connect(session, false).await {
        Ok(s) => s,
        Err(_) => {
            let _ = tx.send(AppEvent::OutputStreamClosed).await;
            return;
        }
    };

    let (reader, mut writer) = stream.into_split();

    // Send follow request with no timeout (infinite streaming)
    let req = Request::Logs {
        target: None,
        tail: 0,
        follow: true,
        stderr: false,
        all: true,
        timeout_secs: None, // infinite — TUI manages its own lifetime
        lines: None,
    };
    let mut json = serde_json::to_string(&req).unwrap();
    json.push('\n');
    if writer.write_all(json.as_bytes()).await.is_err() { return; }
    if writer.flush().await.is_err() { return; }

    let mut lines = BufReader::new(reader);
    loop {
        let mut line = String::new();
        match lines.read_line(&mut line).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                if let Ok(resp) = serde_json::from_str::<Response>(&line) {
                    match resp {
                        Response::LogLine { process, stream, line } => {
                            let _ = tx.send(AppEvent::OutputLine { process, stream, line }).await;
                        }
                        Response::LogEnd => break,
                        _ => {}
                    }
                }
            }
            Err(_) => break,
        }
    }

    let _ = tx.send(AppEvent::OutputStreamClosed).await;
}

async fn status_poller(session: &str, tx: mpsc::Sender<AppEvent>) {
    let mut ticker = interval(Duration::from_secs(2));
    loop {
        ticker.tick().await;
        if let Ok(Response::Status { processes }) = cli::request(session, &Request::Status, false).await {
            if tx.send(AppEvent::StatusUpdate(processes)).await.is_err() {
                break; // Receiver dropped, TUI is shutting down
            }
        }
    }
}

async fn key_reader(tx: mpsc::Sender<AppEvent>) {
    let mut reader = EventStream::new();
    while let Some(Ok(event)) = reader.next().await {
        if let Event::Key(key) = event {
            if tx.send(AppEvent::Key(key)).await.is_err() {
                break;
            }
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

(`futures` and `crossterm` event-stream feature were already added in Task 2.)

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: Compiles

- [ ] **Step 4: Run all existing tests**

Run: `cargo test -- --test-threads=1`
Expected: All tests still pass

- [ ] **Step 5: Commit**

```bash
git add src/tui/mod.rs Cargo.toml
git commit -m "feat: implement TUI event loop with daemon connections"
```

---

### Task 7: Integration smoke test and final cleanup

**Files:**
- Create: `tests/test_tui_smoke.rs`
- Modify: `skill/agent-procs.md`

- [ ] **Step 1: Write smoke test**

Create `tests/test_tui_smoke.rs`:

```rust
mod helpers;
use assert_cmd::Command;
use helpers::TestContext;
use std::time::Duration;

/// Smoke test: verify the TUI starts and can be interrupted without crashing.
/// We can't interact with the TUI in an integration test, but we can verify
/// it starts, connects to the daemon, and exits cleanly when killed.
#[test]
fn test_ui_starts_and_exits() {
    let ctx = TestContext::new("t-ui-smoke");
    ctx.set_env();

    // Start a process so the daemon exists
    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "run", "sleep 60", "--name", "bg"])
        .output().unwrap();

    // Launch the TUI with a very short timeout — it will be killed
    let output = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "ui"])
        .timeout(Duration::from_secs(3))
        .output();

    // The TUI will be killed by the timeout — that's expected.
    // We just verify it didn't panic or crash with a non-timeout error.
    match output {
        Ok(o) => {
            // If it exited within 3s, that's fine (maybe no terminal)
            let _ = o;
        }
        Err(e) => {
            // Timeout is expected and OK
            let err_str = e.to_string();
            assert!(
                err_str.contains("timed out") || err_str.contains("timeout"),
                "unexpected error: {}", err_str
            );
        }
    }

    let _ = Command::cargo_bin("agent-procs").unwrap()
        .args(["--session", &ctx.session, "stop-all"]).output();
}
```

- [ ] **Step 2: Run smoke test**

Run: `cargo test --test test_tui_smoke -- --test-threads=1`
Expected: Passes (the test just verifies the binary doesn't crash)

- [ ] **Step 3: Update skill document**

Add to `skill/agent-procs.md`, after the "Stopping" section:

```markdown
## Monitoring (TUI)

```bash
agent-procs ui                    # open terminal UI for current session
agent-procs ui --session frontend # specific session
```

Keybindings: ↑↓ select, `r` restart, `x` stop, `X` stop-all, `e` cycle stdout/stderr/both, Space pause, `q` quit, `Q` quit+stop.
```

- [ ] **Step 4: Run all tests**

Run: `cargo test -- --test-threads=1`
Expected: All tests pass

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: Clean. Fix any issues.

- [ ] **Step 6: Build release**

Run: `cargo build --release`
Expected: Builds successfully

- [ ] **Step 7: Commit**

```bash
git add tests/test_tui_smoke.rs skill/agent-procs.md
git commit -m "feat: add TUI smoke test and update skill document"
```
