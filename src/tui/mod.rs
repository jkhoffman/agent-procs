pub mod app;
pub mod ui;
pub mod input;

use crate::cli;
use crate::protocol::{Request, Response, Stream as ProtoStream};
use app::App;
use crossterm::{
    event::{Event, EventStream},
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
