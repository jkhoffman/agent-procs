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
