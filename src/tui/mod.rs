//! Terminal UI for real-time process monitoring.
//!
//! The TUI connects to a running daemon session and displays a split-pane
//! layout: a process list on the left and streaming output on the right.
//! It polls status every 2 seconds and streams output via `Logs --follow`.
//!
//! See [`app`] for state management, [`input`] for keybindings, and
//! [`ui`] for rendering.

pub mod app;
pub mod input;
pub mod ui;

use crate::cli;
use crate::paths;
use crate::protocol::{Request, Response, Stream as ProtoStream};
use app::App;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::prelude::*;
use std::io;
use std::io::{BufRead as _, BufReader as StdBufReader};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::time::{Duration, interval};

enum AppEvent {
    Key(crossterm::event::KeyEvent),
    Mouse(crossterm::event::MouseEvent),
    OutputLine {
        process: String,
        stream: ProtoStream,
        line: String,
    },
    StatusUpdate(Vec<crate::protocol::ProcessInfo>),
    OutputStreamClosed,
}

pub async fn run(session: &str) -> i32 {
    // Verify daemon is running
    if cli::connect(session, false).await.is_err() {
        eprintln!(
            "error: no daemon running for session '{}'. Start processes first.",
            session
        );
        return 1;
    }

    // Set up terminal
    if let Err(e) = enable_raw_mode() {
        eprintln!("error: failed to enable raw mode: {}", e);
        return 1;
    }
    let mut stdout = io::stdout();
    if let Err(e) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
        let _ = disable_raw_mode();
        eprintln!("error: failed to enter alternate screen: {}", e);
        return 1;
    }

    // Install panic hook to restore terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    let mut terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
        Ok(t) => t,
        Err(e) => {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            eprintln!("error: failed to initialize terminal: {}", e);
            return 1;
        }
    };
    let mut app = App::new();

    // Load historical output from log files on disk
    load_historical_logs(session, &mut app);

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
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
            let _ = sigterm_tx
                .send(AppEvent::Key(crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('q'),
                    crossterm::event::KeyModifiers::empty(),
                )))
                .await;
        }
    });

    // Track reconnection attempts for backoff
    let reconnect_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
    const MAX_RECONNECT_ATTEMPTS: u32 = 10;

    // Main render loop
    while app.running {
        if let Err(e) = terminal.draw(|f| ui::draw(f, &mut app)) {
            eprintln!("error: terminal draw failed: {}", e);
            break;
        }

        // Wait for next event
        if let Some(event) = rx.recv().await {
            match event {
                AppEvent::Key(key) => {
                    // Route keys based on input mode
                    if app.input_mode == app::InputMode::FilterInput {
                        match input::handle_filter_key(key) {
                            input::FilterAction::Char(c) => app.filter_buf.push(c),
                            input::FilterAction::Backspace => {
                                app.filter_buf.pop();
                            }
                            input::FilterAction::Confirm => app.confirm_filter(),
                            input::FilterAction::Cancel => app.cancel_filter(),
                        }
                    } else {
                        let action = input::handle_key(key);
                        match action {
                            input::Action::SelectNext => app.select_next(),
                            input::Action::SelectPrev => app.select_prev(),
                            input::Action::CycleStream => app.cycle_stream_mode(),
                            input::Action::TogglePause => app.toggle_pause(),
                            input::Action::ScrollUp => app.scroll_up(),
                            input::Action::ScrollDown => app.scroll_down(),
                            input::Action::ScrollToTop => app.scroll_to_top(),
                            input::Action::ScrollToBottom => app.scroll_to_bottom(),
                            input::Action::StartFilter => app.start_filter(),
                            input::Action::ClearFilter => app.clear_filter(),
                            input::Action::Quit => app.quit(),
                            input::Action::QuitAndStop => app.quit_and_stop(),
                            input::Action::Stop => {
                                if let Some(name) = app.selected_name() {
                                    let _ = cli::request(
                                        session,
                                        &Request::Stop {
                                            target: name.to_string(),
                                        },
                                        false,
                                    )
                                    .await;
                                }
                            }
                            input::Action::StopAll => {
                                let _ = cli::request(session, &Request::StopAll, false).await;
                            }
                            input::Action::Restart => {
                                if let Some(name) = app.selected_name() {
                                    let _ = cli::request(
                                        session,
                                        &Request::Restart {
                                            target: name.to_string(),
                                        },
                                        false,
                                    )
                                    .await;
                                }
                            }
                            input::Action::None => {}
                        }
                    }
                }
                AppEvent::Mouse(mouse) => {
                    use crossterm::event::{MouseButton, MouseEventKind};
                    match mouse.kind {
                        MouseEventKind::ScrollUp => app.scroll_up(),
                        MouseEventKind::ScrollDown => app.scroll_down(),
                        MouseEventKind::Down(MouseButton::Left) => {
                            // Click in process list (left 22 columns) to select
                            if mouse.column < 22 {
                                let row = mouse.row.saturating_sub(1) as usize; // -1 for border
                                if row < app.processes.len() {
                                    app.selected = row;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                AppEvent::OutputLine {
                    process,
                    stream,
                    line,
                } => {
                    app.push_output(&process, stream, &line);
                }
                AppEvent::StatusUpdate(processes) => {
                    app.update_processes(processes);
                }
                AppEvent::OutputStreamClosed => {
                    let count = reconnect_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if count < MAX_RECONNECT_ATTEMPTS {
                        let session_str = session.to_string();
                        let reconnect_tx = tx.clone();
                        let rc = Arc::clone(&reconnect_count);
                        tokio::spawn(async move {
                            // Exponential backoff: 2s, 4s, 8s, ... capped at 30s
                            let delay = Duration::from_secs(2u64.saturating_pow(count).min(30));
                            tokio::time::sleep(delay).await;
                            output_stream_reader(&session_str, reconnect_tx).await;
                            // Reset counter on successful reconnection
                            rc.store(0, std::sync::atomic::Ordering::Relaxed);
                        });
                    }
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
    let _ = execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    );
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
    let mut json = match serde_json::to_string(&req) {
        Ok(j) => j,
        Err(_) => return,
    };
    json.push('\n');
    if writer.write_all(json.as_bytes()).await.is_err() {
        return;
    }
    if writer.flush().await.is_err() {
        return;
    }

    let mut lines = BufReader::new(reader);
    loop {
        let mut line = String::new();
        match lines.read_line(&mut line).await {
            Ok(0) | Err(_) => break, // EOF or error
            Ok(_) => {
                if let Ok(resp) = serde_json::from_str::<Response>(&line) {
                    match resp {
                        Response::LogLine {
                            process,
                            stream,
                            line,
                        } => {
                            let _ = tx
                                .send(AppEvent::OutputLine {
                                    process,
                                    stream,
                                    line,
                                })
                                .await;
                        }
                        Response::LogEnd => break,
                        _ => {}
                    }
                }
            }
        }
    }

    let _ = tx.send(AppEvent::OutputStreamClosed).await;
}

async fn status_poller(session: &str, tx: mpsc::Sender<AppEvent>) {
    let mut ticker = interval(Duration::from_secs(2));
    loop {
        ticker.tick().await;
        if let Ok(Response::Status { processes }) =
            cli::request(session, &Request::Status, false).await
        {
            if tx.send(AppEvent::StatusUpdate(processes)).await.is_err() {
                break; // Receiver dropped, TUI is shutting down
            }
        }
    }
}

async fn key_reader(tx: mpsc::Sender<AppEvent>) {
    let mut reader = EventStream::new();
    while let Some(Ok(event)) = reader.next().await {
        let app_event = match event {
            Event::Key(key) => AppEvent::Key(key),
            Event::Mouse(mouse) => AppEvent::Mouse(mouse),
            _ => continue,
        };
        if tx.send(app_event).await.is_err() {
            break;
        }
    }
}

/// Load the tail of each process's log files from disk into the app's output buffers.
/// This populates the TUI with historical output from before it was launched.
fn load_historical_logs(session: &str, app: &mut App) {
    let log_dir = paths::log_dir(session);

    let entries = match std::fs::read_dir(&log_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let (proc_name, stream) = if let Some(p) = name.strip_suffix(".stdout") {
            (p.to_string(), ProtoStream::Stdout)
        } else if let Some(p) = name.strip_suffix(".stderr") {
            (p.to_string(), ProtoStream::Stderr)
        } else {
            continue;
        };

        if let Ok(file) = std::fs::File::open(entry.path()) {
            for line in StdBufReader::new(file).lines().map_while(Result::ok) {
                app.push_output(&proc_name, stream, &line);
            }
        }
    }
}
