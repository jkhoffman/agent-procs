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

impl Default for App {
    fn default() -> Self { Self::new() }
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
