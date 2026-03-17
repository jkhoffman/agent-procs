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
    ScrollUp,
    ScrollDown,
    ScrollToTop,
    ScrollToBottom,
    StartFilter,
    ClearFilter,
    Quit,
    QuitAndStop,
    None,
}

/// Actions available when the filter input prompt is active.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterAction {
    Char(char),
    Backspace,
    Confirm,
    Cancel,
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
        KeyCode::PageUp | KeyCode::Char('u') => Action::ScrollUp,
        KeyCode::PageDown | KeyCode::Char('d') => Action::ScrollDown,
        KeyCode::Home | KeyCode::Char('g') => Action::ScrollToTop,
        KeyCode::End | KeyCode::Char('G') => Action::ScrollToBottom,
        KeyCode::Char('/') => Action::StartFilter,
        KeyCode::Esc => Action::ClearFilter,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('Q') => Action::QuitAndStop,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        _ => Action::None,
    }
}

pub fn handle_filter_key(key: KeyEvent) -> FilterAction {
    match key.code {
        KeyCode::Enter => FilterAction::Confirm,
        KeyCode::Backspace => FilterAction::Backspace,
        KeyCode::Char(c) => FilterAction::Char(c),
        _ => FilterAction::Cancel, // Esc or any unrecognized key cancels
    }
}
