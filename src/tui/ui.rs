use crate::protocol::ProcessState;
use crate::tui::app::{App, InputMode, LineSource, StreamMode};
use ansi_to_tui::IntoText;
use ratatui::prelude::*;
use ratatui::widgets::*;

pub fn draw(frame: &mut Frame, app: &mut App) {
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
            Constraint::Min(30),    // output
        ])
        .split(chunks[0]);

    draw_process_list(frame, app, main_chunks[0]);
    draw_output(frame, app, main_chunks[1]);
    draw_status_bar(frame, app, chunks[1]);
}

fn draw_process_list(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .processes
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (indicator, style) = match p.state {
                ProcessState::Running => ("●", Style::default().fg(Color::Green)),
                ProcessState::Exited => {
                    if p.exit_code == Some(0) {
                        ("✓", Style::default().fg(Color::DarkGray))
                    } else {
                        let code = p.exit_code.map(|c| format!(" ({})", c)).unwrap_or_default();
                        return ListItem::new(format!("✗ {}{}", p.name, code)).style(
                            if i == app.selected {
                                Style::default().fg(Color::Red).bg(Color::DarkGray)
                            } else {
                                Style::default().fg(Color::Red)
                            },
                        );
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
        })
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(" Processes "));

    frame.render_widget(list, area);
}

/// Convert a line of text (possibly containing ANSI escape codes) to a ratatui Line.
/// Falls back to plain text if ANSI parsing fails.
fn ansi_line(text: &str, fallback_style: Option<Style>) -> Line<'static> {
    match text.as_bytes().into_text() {
        Ok(parsed) => {
            // into_text returns a Text (multiple lines); take the first line
            if let Some(line) = parsed.lines.into_iter().next() {
                line
            } else {
                Line::from(text.to_string())
            }
        }
        Err(_) => {
            let line = Line::from(text.to_string());
            if let Some(style) = fallback_style {
                line.style(style)
            } else {
                line
            }
        }
    }
}

fn draw_output(frame: &mut Frame, app: &mut App, area: Rect) {
    let name = app.selected_name().unwrap_or("(none)").to_string();
    let mode_label = match app.stream_mode {
        StreamMode::Stdout => "stdout",
        StreamMode::Stderr => "stderr",
        StreamMode::Both => "all",
    };
    let pause_indicator = if app.paused { " [PAUSED]" } else { "" };
    let filter_indicator = match (&app.input_mode, &app.filter) {
        (InputMode::FilterInput, _) => format!(" /{}", app.filter_buf),
        (_, Some(pat)) => format!(" [filter: {}]", pat),
        _ => String::new(),
    };
    let title = format!(
        " Output: {} ({}){}{}  ",
        name, mode_label, pause_indicator, filter_indicator
    );

    // Cache visible height for scroll calculations
    let visible_height = area.height.saturating_sub(2) as usize; // -2 for borders
    app.visible_height = visible_height;

    let stderr_style = Style::default().fg(Color::Yellow);

    // Try windowed rendering (disk-backed). Falls back to None when filter is active.
    if let Some(window) = app.visible_lines(&name, visible_height) {
        let lines: Vec<Line> = if window.is_empty() && !app.buffers.contains_key(&name) {
            vec![
                Line::from("No output yet".to_string()).style(Style::default().fg(Color::DarkGray)),
            ]
        } else {
            window
                .iter()
                .map(|(src, l)| {
                    let style = if *src == LineSource::Stderr {
                        Some(stderr_style)
                    } else {
                        None
                    };
                    ansi_line(l, style)
                })
                .collect()
        };

        let paragraph =
            Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title));
        frame.render_widget(paragraph, area);
    } else {
        // Filtered mode: fall back to collect-all-from-hot-buffer approach
        let filter_pat = app.filter.as_deref();
        let matches_filter = |line: &str| filter_pat.is_none_or(|p| line.contains(p));

        let lines: Vec<Line> = if let Some(buf) = app.buffers.get(&name) {
            match app.stream_mode {
                StreamMode::Stdout => buf
                    .stdout_lines()
                    .filter(|l| matches_filter(l))
                    .map(|l| ansi_line(l, None))
                    .collect(),
                StreamMode::Stderr => buf
                    .stderr_lines()
                    .filter(|l| matches_filter(l))
                    .map(|l| ansi_line(l, Some(stderr_style)))
                    .collect(),
                StreamMode::Both => buf
                    .all_lines()
                    .filter(|(_, l)| matches_filter(l))
                    .map(|(src, l)| {
                        let style = if src == LineSource::Stderr {
                            Some(stderr_style)
                        } else {
                            None
                        };
                        ansi_line(l, style)
                    })
                    .collect(),
            }
        } else {
            vec![
                Line::from("No output yet".to_string()).style(Style::default().fg(Color::DarkGray)),
            ]
        };

        let total_lines = lines.len();
        let scroll_offset = if app.paused {
            app.scroll_offsets
                .get(name.as_str())
                .copied()
                .unwrap_or(0)
                .min(total_lines.saturating_sub(visible_height))
        } else {
            0
        };
        let scroll_pos = total_lines.saturating_sub(visible_height + scroll_offset);

        let paragraph = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .scroll((scroll_pos as u16, 0));
        frame.render_widget(paragraph, area);
    }
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let keys = if app.input_mode == InputMode::FilterInput {
        " type to filter, Enter confirm, Esc cancel ".to_string()
    } else if app.paused {
        " PgUp/u scroll up  PgDn/d scroll down  g top  G bottom  Space unpause  / filter  Esc clear filter ".to_string()
    } else {
        " ↑↓ select  r restart  x stop  X stop-all  e stream  Space pause  u/d scroll  / filter  q quit  Q down ".to_string()
    };

    let counts = format!(
        " {} running, {} exited ",
        app.running_count(),
        app.exited_count()
    );

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
