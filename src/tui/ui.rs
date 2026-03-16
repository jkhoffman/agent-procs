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
