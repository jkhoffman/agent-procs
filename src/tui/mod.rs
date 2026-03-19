//! Terminal UI for real-time process monitoring.
//!
//! The TUI connects to a running daemon session and displays a split-pane
//! layout: a process list on the left and streaming output on the right.
//! It polls status every 2 seconds and streams output via `Logs --follow`.
//!
//! See [`app`] for state management, [`input`] for keybindings, and
//! [`ui`] for rendering.

pub mod app;
pub use crate::disk_log_reader;
pub mod event_loop;
pub mod input;
pub mod ui;

use crate::cli;
use app::App;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use std::io;

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

    // Initialize disk readers and load recent history into hot buffer
    event_loop::init_disk_readers(session, &mut app);

    // Create event loop and spawn background tasks
    let mut event_loop = event_loop::TuiEventLoop::new(session);
    event_loop.spawn_background_tasks();

    // Run the main event loop
    event_loop.run(&mut terminal, &mut app).await;

    // Restore terminal
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    );
    0
}
