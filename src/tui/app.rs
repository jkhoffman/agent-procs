use crate::protocol::{ProcessInfo, Stream};
use std::collections::{HashMap, VecDeque};

const MAX_BUFFER_LINES: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StreamMode {
    Stdout,
    Stderr,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineSource {
    Stdout,
    Stderr,
}

/// Single ring buffer storing all output with source tags.
/// Stdout/stderr views are filtered from the same data — no duplication.
pub struct OutputBuffer {
    lines: VecDeque<(LineSource, String)>,
    max_lines: usize,
}

impl OutputBuffer {
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(max_lines),
            max_lines,
        }
    }

    pub fn push(&mut self, source: LineSource, line: String) {
        if self.lines.len() == self.max_lines {
            self.lines.pop_front();
        }
        self.lines.push_back((source, line));
    }

    pub fn stdout_lines(&self) -> Vec<&str> {
        self.lines
            .iter()
            .filter(|(src, _)| *src == LineSource::Stdout)
            .map(|(_, s)| s.as_str())
            .collect()
    }

    pub fn stderr_lines(&self) -> Vec<&str> {
        self.lines
            .iter()
            .filter(|(src, _)| *src == LineSource::Stderr)
            .map(|(_, s)| s.as_str())
            .collect()
    }

    pub fn all_lines(&self) -> Vec<(LineSource, &str)> {
        self.lines
            .iter()
            .map(|(src, s)| (*src, s.as_str()))
            .collect()
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

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
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
            if let Some(name) = self.processes.get(self.selected).map(|p| p.name.clone()) {
                self.scroll_offsets.remove(&name);
            }
        }
    }

    pub fn push_output(&mut self, process: &str, stream: Stream, line: &str) {
        let buf = self
            .buffers
            .entry(process.to_string())
            .or_insert_with(|| OutputBuffer::new(MAX_BUFFER_LINES));
        let source = match stream {
            Stream::Stdout => LineSource::Stdout,
            Stream::Stderr => LineSource::Stderr,
        };
        buf.push(source, line.to_string());
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn quit_and_stop(&mut self) {
        self.stop_all_on_quit = true;
        self.running = false;
    }

    pub fn running_count(&self) -> usize {
        self.processes
            .iter()
            .filter(|p| p.state == crate::protocol::ProcessState::Running)
            .count()
    }

    pub fn exited_count(&self) -> usize {
        self.processes
            .iter()
            .filter(|p| p.state == crate::protocol::ProcessState::Exited)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ProcessInfo, ProcessState, Stream};

    fn make_process(name: &str, state: ProcessState) -> ProcessInfo {
        let exit_code = if state == ProcessState::Exited {
            Some(0)
        } else {
            None
        };
        let uptime_secs = if state == ProcessState::Running {
            Some(10)
        } else {
            None
        };
        ProcessInfo {
            name: name.into(),
            id: format!("p-{}", name),
            pid: 100,
            state,
            exit_code,
            uptime_secs,
            command: "true".into(),
            port: None,
            url: None,
        }
    }

    #[test]
    fn test_select_next_wraps() {
        let mut app = App::new();
        app.update_processes(vec![
            make_process("a", ProcessState::Running),
            make_process("b", ProcessState::Running),
            make_process("c", ProcessState::Running),
        ]);
        app.selected = 2; // last item
        app.select_next();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_select_prev_wraps() {
        let mut app = App::new();
        app.update_processes(vec![
            make_process("a", ProcessState::Running),
            make_process("b", ProcessState::Running),
            make_process("c", ProcessState::Running),
        ]);
        app.selected = 0;
        app.select_prev();
        assert_eq!(app.selected, 2);
    }

    #[test]
    fn test_cycle_stream_mode() {
        let mut app = App::new();
        assert_eq!(app.stream_mode, StreamMode::Stdout);
        app.cycle_stream_mode();
        assert_eq!(app.stream_mode, StreamMode::Stderr);
        app.cycle_stream_mode();
        assert_eq!(app.stream_mode, StreamMode::Both);
        app.cycle_stream_mode();
        assert_eq!(app.stream_mode, StreamMode::Stdout);
    }

    #[test]
    fn test_toggle_pause() {
        let mut app = App::new();
        assert!(!app.paused);
        app.toggle_pause();
        assert!(app.paused);
        app.toggle_pause();
        assert!(!app.paused);
    }

    #[test]
    fn test_push_output() {
        let mut app = App::new();
        app.push_output("web", Stream::Stdout, "hello world");
        let buf = app.buffers.get("web").unwrap();
        let lines = buf.stdout_lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "hello world");
    }

    #[test]
    fn test_running_count() {
        let mut app = App::new();
        app.update_processes(vec![
            make_process("a", ProcessState::Running),
            make_process("b", ProcessState::Exited),
            make_process("c", ProcessState::Running),
        ]);
        assert_eq!(app.running_count(), 2);
    }

    #[test]
    fn test_exited_count() {
        let mut app = App::new();
        app.update_processes(vec![
            make_process("a", ProcessState::Running),
            make_process("b", ProcessState::Exited),
            make_process("c", ProcessState::Exited),
        ]);
        assert_eq!(app.exited_count(), 2);
    }
}
