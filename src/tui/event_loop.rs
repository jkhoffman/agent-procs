use super::app::App;
use super::input;
use crate::cli;
use crate::cli::logs::tail_file;
use crate::paths;
use crate::protocol::{Request, Response, Stream as ProtoStream};
use crossterm::event::{Event, EventStream, MouseButton, MouseEventKind};
use futures::StreamExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::time::{Duration, interval};

pub enum AppEvent {
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

enum ReconnectState {
    Connected,
    Reconnecting { attempt: u32 },
    Failed,
}

const MAX_RECONNECT_ATTEMPTS: u32 = 10;

pub struct TuiEventLoop {
    rx: mpsc::Receiver<AppEvent>,
    tx: mpsc::Sender<AppEvent>,
    session: String,
    reconnect_state: ReconnectState,
}

impl TuiEventLoop {
    pub fn new(session: &str) -> Self {
        let (tx, rx) = mpsc::channel::<AppEvent>(256);
        Self {
            rx,
            tx,
            session: session.to_string(),
            reconnect_state: ReconnectState::Connected,
        }
    }

    /// Spawn all background tasks (output stream, status poller, key reader, SIGTERM).
    pub fn spawn_background_tasks(&self) {
        let session_str = self.session.clone();
        let output_tx = self.tx.clone();
        tokio::spawn(async move {
            output_stream_reader(&session_str, output_tx).await;
        });

        let session_str = self.session.clone();
        let status_tx = self.tx.clone();
        tokio::spawn(async move {
            status_poller(&session_str, status_tx).await;
        });

        let key_tx = self.tx.clone();
        tokio::spawn(async move {
            key_reader(key_tx).await;
        });

        let sigterm_tx = self.tx.clone();
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
    }

    /// Run the main event loop, dispatching events to the app and drawing the terminal.
    pub async fn run(
        &mut self,
        terminal: &mut ratatui::Terminal<ratatui::prelude::CrosstermBackend<std::io::Stdout>>,
        app: &mut App,
    ) {
        while app.running {
            if let Err(e) = terminal.draw(|f| super::ui::draw(f, app)) {
                eprintln!("error: terminal draw failed: {}", e);
                break;
            }

            if let Some(event) = self.rx.recv().await {
                match event {
                    AppEvent::Key(key) => {
                        self.handle_key(app, key).await;
                    }
                    AppEvent::Mouse(mouse) => {
                        Self::handle_mouse(app, mouse);
                    }
                    AppEvent::OutputLine {
                        process,
                        stream,
                        line,
                    } => {
                        self.reconnect_state = ReconnectState::Connected;
                        app.push_output(&process, stream, &line);
                    }
                    AppEvent::StatusUpdate(processes) => {
                        app.update_processes(processes);
                    }
                    AppEvent::OutputStreamClosed => {
                        self.handle_reconnect();
                    }
                }
            }
        }

        if app.stop_all_on_quit {
            let _ = cli::request(&self.session, &Request::Shutdown, false).await;
        }
    }

    async fn handle_key(&self, app: &mut App, key: crossterm::event::KeyEvent) {
        if app.input_mode == super::app::InputMode::FilterInput {
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
                            &self.session,
                            &Request::Stop {
                                target: name.to_string(),
                            },
                            false,
                        )
                        .await;
                    }
                }
                input::Action::StopAll => {
                    let _ = cli::request(&self.session, &Request::StopAll, false).await;
                }
                input::Action::Restart => {
                    if let Some(name) = app.selected_name() {
                        let _ = cli::request(
                            &self.session,
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

    fn handle_mouse(app: &mut App, mouse: crossterm::event::MouseEvent) {
        const MOUSE_SCROLL_LINES: usize = 3;
        match mouse.kind {
            MouseEventKind::ScrollUp => app.scroll_up_by(MOUSE_SCROLL_LINES),
            MouseEventKind::ScrollDown => app.scroll_down_by(MOUSE_SCROLL_LINES),
            MouseEventKind::Down(MouseButton::Left) => {
                if mouse.column < 22 {
                    let row = mouse.row.saturating_sub(1) as usize;
                    if row < app.processes.len() {
                        app.selected = row;
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_reconnect(&mut self) {
        let attempt = match self.reconnect_state {
            ReconnectState::Connected => 0,
            ReconnectState::Reconnecting { attempt } => attempt,
            ReconnectState::Failed => return,
        };

        if attempt >= MAX_RECONNECT_ATTEMPTS {
            self.reconnect_state = ReconnectState::Failed;
            return;
        }

        self.reconnect_state = ReconnectState::Reconnecting {
            attempt: attempt + 1,
        };

        let session_str = self.session.clone();
        let reconnect_tx = self.tx.clone();
        tokio::spawn(async move {
            let delay = Duration::from_secs(2u64.saturating_pow(attempt).min(30));
            tokio::time::sleep(delay).await;
            output_stream_reader(&session_str, reconnect_tx).await;
        });
    }
}

pub async fn output_stream_reader(session: &str, tx: mpsc::Sender<AppEvent>) {
    let stream = match cli::connect(session, false).await {
        Ok(s) => s,
        Err(_) => {
            let _ = tx.send(AppEvent::OutputStreamClosed).await;
            return;
        }
    };

    let (reader, mut writer) = stream.into_split();

    let req = Request::Logs {
        target: None,
        tail: 0,
        follow: true,
        stderr: false,
        all: true,
        timeout_secs: None,
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
            Ok(0) | Err(_) => break,
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

pub async fn status_poller(session: &str, tx: mpsc::Sender<AppEvent>) {
    let mut ticker = interval(Duration::from_secs(2));
    loop {
        ticker.tick().await;
        if let Ok(Response::Status { processes }) =
            cli::request(session, &Request::Status, false).await
            && tx.send(AppEvent::StatusUpdate(processes)).await.is_err()
        {
            break;
        }
    }
}

pub async fn key_reader(tx: mpsc::Sender<AppEvent>) {
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
/// Uses `tail_file` to read only the last 1000 lines instead of entire files.
pub fn load_historical_logs(session: &str, app: &mut App) {
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

        if let Ok(lines) = tail_file(&entry.path(), 1000) {
            for line in lines {
                app.push_output(&proc_name, stream, &line);
            }
        }
    }
}
