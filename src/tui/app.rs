use crate::disk_log_reader::DiskLogReader;
use crate::protocol::{ProcessInfo, Stream};
use std::collections::{HashMap, VecDeque};

pub use crate::disk_log_reader::{LineSource, StreamMode};

const MAX_BUFFER_LINES: usize = 10_000;

/// Single ring buffer storing all output with source tags.
/// Stdout/stderr views are filtered from the same data — no duplication.
pub struct OutputBuffer {
    lines: VecDeque<(LineSource, String)>,
    max_lines: usize,
    stdout_count: usize,
    stderr_count: usize,
}

impl OutputBuffer {
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(max_lines),
            max_lines,
            stdout_count: 0,
            stderr_count: 0,
        }
    }

    pub fn push(&mut self, source: LineSource, line: String) {
        if self.lines.len() == self.max_lines
            && let Some((evicted, _)) = self.lines.pop_front()
        {
            match evicted {
                LineSource::Stdout => self.stdout_count -= 1,
                LineSource::Stderr => self.stderr_count -= 1,
            }
        }
        match source {
            LineSource::Stdout => self.stdout_count += 1,
            LineSource::Stderr => self.stderr_count += 1,
        }
        self.lines.push_back((source, line));
    }

    /// O(1) count of total lines.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// O(1) count of stdout lines.
    pub fn stdout_count(&self) -> usize {
        self.stdout_count
    }

    /// O(1) count of stderr lines.
    pub fn stderr_count(&self) -> usize {
        self.stderr_count
    }

    pub fn stdout_lines(&self) -> impl Iterator<Item = &str> {
        self.lines
            .iter()
            .filter(|(src, _)| *src == LineSource::Stdout)
            .map(|(_, s)| s.as_str())
    }

    pub fn stderr_lines(&self) -> impl Iterator<Item = &str> {
        self.lines
            .iter()
            .filter(|(src, _)| *src == LineSource::Stderr)
            .map(|(_, s)| s.as_str())
    }

    pub fn all_lines(&self) -> impl Iterator<Item = (LineSource, &str)> {
        self.lines.iter().map(|(src, s)| (*src, s.as_str()))
    }
}

/// Cached index of matching line numbers for filtered disk scrollback.
pub struct FilteredIndex {
    pub filter: String,
    pub stream_mode: StreamMode,
    /// Line indices (in the stream mode's address space) that match the filter.
    pub matching_lines: Vec<usize>,
    /// Number of disk lines scanned so far (for incremental updates).
    pub scanned_up_to: usize,
}

/// Input mode for the TUI.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputMode {
    /// Normal keybinding mode.
    Normal,
    /// Typing a filter pattern.
    FilterInput,
}

#[allow(clippy::struct_excessive_bools)]
pub struct App {
    pub processes: Vec<ProcessInfo>,
    pub selected: usize,
    pub buffers: HashMap<String, OutputBuffer>,
    pub stream_mode: StreamMode,
    pub paused: bool,
    pub scroll_offsets: HashMap<String, usize>,
    pub running: bool,
    pub stop_all_on_quit: bool,
    pub input_mode: InputMode,
    /// In-progress filter text while the user is typing.
    pub filter_buf: String,
    /// Active filter applied to output lines. `None` means no filter.
    pub filter: Option<String>,
    /// Cached visible height of the output pane (set during render).
    pub visible_height: usize,
    /// Disk-backed log readers for each process.
    pub disk_readers: HashMap<String, DiskLogReader>,
    /// Cached filtered indices for filtered disk scrollback.
    pub filtered_indices: HashMap<String, FilteredIndex>,
    /// Whether the filter uses regex matching (true) or substring matching (false).
    pub filter_regex: bool,
    /// Whether the current regex pattern is invalid.
    pub filter_regex_error: bool,
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
            input_mode: InputMode::Normal,
            filter_buf: String::new(),
            filter: None,
            visible_height: 20,
            disk_readers: HashMap::new(),
            filtered_indices: HashMap::new(),
            filter_regex: false,
            filter_regex_error: false,
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
        // Rebuild filtered indices if filter is active (different line numbering)
        if self.filter.is_some() {
            let names: Vec<String> = self.disk_readers.keys().cloned().collect();
            for name in names {
                self.build_filtered_index(&name);
            }
        }
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

    /// Scroll up by the given number of lines. Auto-pauses if not already paused.
    /// Clamps to the maximum scrollable range so overshooting the top is impossible.
    pub fn scroll_up_by(&mut self, lines: usize) {
        if !self.paused {
            self.paused = true;
        }
        if let Some(name) = self.selected_name().map(str::to_string) {
            let max_offset = self
                .line_count_for(&name)
                .saturating_sub(self.visible_height);
            let offset = self.scroll_offsets.entry(name).or_insert(0);
            *offset = offset.saturating_add(lines).min(max_offset);
        }
    }

    /// Scroll up by half a page.
    pub fn scroll_up(&mut self) {
        let half_page = (self.visible_height / 2).max(1);
        self.scroll_up_by(half_page);
    }

    /// Scroll down by the given number of lines. If we reach the bottom, unpause.
    pub fn scroll_down_by(&mut self, lines: usize) {
        if let Some(name) = self.selected_name().map(str::to_string) {
            let offset = self.scroll_offsets.entry(name).or_insert(0);
            *offset = offset.saturating_sub(lines);
            if *offset == 0 {
                self.paused = false;
            }
        }
    }

    /// Scroll down by half a page.
    pub fn scroll_down(&mut self) {
        let half_page = (self.visible_height / 2).max(1);
        self.scroll_down_by(half_page);
    }

    pub fn scroll_to_top(&mut self) {
        if !self.paused {
            self.paused = true;
        }
        if let Some(name) = self.selected_name().map(str::to_string) {
            let total = self.line_count_for(&name);
            self.scroll_offsets.insert(name, total);
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        if let Some(name) = self.selected_name().map(str::to_string) {
            self.scroll_offsets.remove(&name);
            self.paused = false;
        }
    }

    /// Count visible lines for the selected process.
    /// When a filter is active, uses the filtered index count.
    /// Otherwise, uses the disk-backed total (authoritative).
    fn line_count_for(&mut self, name: &str) -> usize {
        if self.filter.is_some() {
            return self
                .filtered_indices
                .get(name)
                .map_or(0, |fi| fi.matching_lines.len());
        }
        self.total_line_count(name)
    }

    /// Total line count combining disk history and hot buffer.
    /// Uses disk as authoritative; falls back to hot buffer if larger
    /// (e.g. right after a process restart before disk catches up).
    fn total_line_count(&mut self, name: &str) -> usize {
        let hot = self.hot_line_count(name, None);
        let disk = self.disk_line_count(name);
        disk.max(hot)
    }

    /// Disk line count for the current stream mode.
    fn disk_line_count(&mut self, name: &str) -> usize {
        let mode = self.stream_mode;
        self.disk_readers.get_mut(name).map_or(0, |r| match mode {
            StreamMode::Stdout => r.line_count(LineSource::Stdout),
            StreamMode::Stderr => r.line_count(LineSource::Stderr),
            StreamMode::Both => r.line_count_both(),
        })
    }

    /// Hot buffer line count, optionally filtered by a substring pattern.
    /// O(1) when `filter` is `None`; O(n) when filtering.
    fn hot_line_count(&self, name: &str, filter: Option<&str>) -> usize {
        let Some(buf) = self.buffers.get(name) else {
            return 0;
        };
        if let Some(pat) = filter {
            let matches = |line: &str| line.contains(pat);
            return match self.stream_mode {
                StreamMode::Stdout => buf.stdout_lines().filter(|l| matches(l)).count(),
                StreamMode::Stderr => buf.stderr_lines().filter(|l| matches(l)).count(),
                StreamMode::Both => buf.all_lines().filter(|(_, l)| matches(l)).count(),
            };
        }
        match self.stream_mode {
            StreamMode::Stdout => buf.stdout_count(),
            StreamMode::Stderr => buf.stderr_count(),
            StreamMode::Both => buf.len(),
        }
    }

    /// Fetch exactly the visible window of lines for rendering.
    ///
    /// When a filter is active, uses the `FilteredIndex` to read matching
    /// lines from disk. Returns `Some` in all cases.
    pub fn visible_lines(
        &mut self,
        name: &str,
        visible_height: usize,
    ) -> Option<Vec<(LineSource, String)>> {
        if self.filter.is_some() {
            return Some(self.filtered_visible_lines(name, visible_height));
        }

        let total = self.total_line_count(name);
        let scroll_offset = if self.paused {
            self.scroll_offsets.get(name).copied().unwrap_or(0)
        } else {
            0
        };

        let window_end = total.saturating_sub(scroll_offset);
        let window_start = window_end.saturating_sub(visible_height);
        let count = window_end - window_start;

        if count == 0 {
            return Some(Vec::new());
        }

        let hot_len = self.hot_line_count(name, None);
        let disk_count = self.disk_line_count(name);
        // Boundary: lines before this come from disk, at or after from hot buffer.
        let disk_boundary = disk_count.saturating_sub(hot_len);

        if window_start >= disk_boundary {
            // Entire window in hot buffer
            let hot_start = window_start - disk_boundary;
            Some(self.hot_buffer_range(name, hot_start, count))
        } else if window_end <= disk_boundary {
            // Entire window on disk
            Some(self.disk_read_range(name, window_start, count))
        } else {
            // Split at boundary
            let disk_part = disk_boundary - window_start;
            let hot_part = window_end - disk_boundary;
            let mut lines = self.disk_read_range(name, window_start, disk_part);
            lines.extend(self.hot_buffer_range(name, 0, hot_part));
            Some(lines)
        }
    }

    /// Read a range from the hot buffer (no filter).
    fn hot_buffer_range(
        &self,
        name: &str,
        start: usize,
        count: usize,
    ) -> Vec<(LineSource, String)> {
        let Some(buf) = self.buffers.get(name) else {
            return Vec::new();
        };
        match self.stream_mode {
            StreamMode::Stdout => buf
                .stdout_lines()
                .skip(start)
                .take(count)
                .map(|l| (LineSource::Stdout, l.to_string()))
                .collect(),
            StreamMode::Stderr => buf
                .stderr_lines()
                .skip(start)
                .take(count)
                .map(|l| (LineSource::Stderr, l.to_string()))
                .collect(),
            StreamMode::Both => buf
                .all_lines()
                .skip(start)
                .take(count)
                .map(|(src, l)| (src, l.to_string()))
                .collect(),
        }
    }

    /// Read a range from the disk reader.
    fn disk_read_range(
        &mut self,
        name: &str,
        start: usize,
        count: usize,
    ) -> Vec<(LineSource, String)> {
        let Some(reader) = self.disk_readers.get_mut(name) else {
            return Vec::new();
        };
        match self.stream_mode {
            StreamMode::Stdout => reader
                .read_lines(LineSource::Stdout, start, count)
                .unwrap_or_default()
                .into_iter()
                .map(|l| (LineSource::Stdout, l))
                .collect(),
            StreamMode::Stderr => reader
                .read_lines(LineSource::Stderr, start, count)
                .unwrap_or_default()
                .into_iter()
                .map(|l| (LineSource::Stderr, l))
                .collect(),
            StreamMode::Both => reader.read_interleaved(start, count).unwrap_or_default(),
        }
    }

    /// Render the visible window of filtered lines using the `FilteredIndex`.
    fn filtered_visible_lines(
        &mut self,
        name: &str,
        visible_height: usize,
    ) -> Vec<(LineSource, String)> {
        let total = self
            .filtered_indices
            .get(name)
            .map_or(0, |fi| fi.matching_lines.len());
        if total == 0 {
            return Vec::new();
        }

        let scroll_offset = if self.paused {
            self.scroll_offsets.get(name).copied().unwrap_or(0)
        } else {
            0
        };

        let window_end = total.saturating_sub(scroll_offset);
        let window_start = window_end.saturating_sub(visible_height);
        let count = window_end - window_start;
        if count == 0 {
            return Vec::new();
        }

        // Get the line numbers we need to read
        let line_numbers: Vec<usize> = self
            .filtered_indices
            .get(name)
            .map(|fi| fi.matching_lines[window_start..window_end].to_vec())
            .unwrap_or_default();

        let mode = self.stream_mode;

        // Read scattered lines from disk
        if let Some(reader) = self.disk_readers.get_mut(name) {
            reader.read_scattered_lines(mode, &line_numbers)
        } else {
            Vec::new()
        }
    }

    /// Build or rebuild the filtered index for a process.
    fn build_filtered_index(&mut self, name: &str) {
        use crate::disk_log_reader::MatchMode;

        let Some(filter) = self.filter.clone() else {
            return;
        };
        let mode = self.stream_mode;

        let match_mode = if self.filter_regex {
            match regex::Regex::new(&filter) {
                Ok(re) => {
                    self.filter_regex_error = false;
                    MatchMode::Regex(re)
                }
                Err(_) => {
                    self.filter_regex_error = true;
                    self.filtered_indices.remove(name);
                    return;
                }
            }
        } else {
            self.filter_regex_error = false;
            MatchMode::Substring(filter.clone())
        };

        let matching_lines = if let Some(reader) = self.disk_readers.get_mut(name) {
            reader.scan_matching_lines_mode(&match_mode, mode)
        } else {
            Vec::new()
        };

        let scanned_up_to = match mode {
            StreamMode::Stdout => self
                .disk_readers
                .get_mut(name)
                .map_or(0, |r| r.line_count(LineSource::Stdout)),
            StreamMode::Stderr => self
                .disk_readers
                .get_mut(name)
                .map_or(0, |r| r.line_count(LineSource::Stderr)),
            StreamMode::Both => self
                .disk_readers
                .get_mut(name)
                .map_or(0, DiskLogReader::line_count_both),
        };

        self.filtered_indices.insert(
            name.to_string(),
            FilteredIndex {
                filter,
                stream_mode: mode,
                matching_lines,
                scanned_up_to,
            },
        );
    }

    pub fn start_filter(&mut self) {
        self.input_mode = InputMode::FilterInput;
        self.filter_buf = self.filter.clone().unwrap_or_default();
    }

    pub fn confirm_filter(&mut self) {
        self.input_mode = InputMode::Normal;
        if self.filter_buf.is_empty() {
            self.filter = None;
            self.filtered_indices.clear();
        } else {
            self.filter = Some(self.filter_buf.clone());
            // Build filtered index for all processes with disk readers
            let names: Vec<String> = self.disk_readers.keys().cloned().collect();
            for name in names {
                self.build_filtered_index(&name);
            }
        }
    }

    pub fn cancel_filter(&mut self) {
        self.input_mode = InputMode::Normal;
        self.filter_buf.clear();
    }

    pub fn clear_filter(&mut self) {
        self.filter = None;
        self.filter_buf.clear();
        self.filtered_indices.clear();
        self.filter_regex = false;
        self.filter_regex_error = false;
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

        // Incrementally update filtered index if a filter is active
        if self.filter.is_some() {
            self.update_filtered_index(process);
        }
    }

    /// Scan new disk lines since last scan and append matches to the filtered index.
    fn update_filtered_index(&mut self, process: &str) {
        use crate::disk_log_reader::MatchMode;

        let Some(fi) = self.filtered_indices.get(process) else {
            return;
        };
        let filter = fi.filter.clone();
        let mode = fi.stream_mode;
        let scanned_up_to = fi.scanned_up_to;

        let match_mode = if self.filter_regex {
            match regex::Regex::new(&filter) {
                Ok(re) => MatchMode::Regex(re),
                Err(_) => return, // already flagged in build
            }
        } else {
            MatchMode::Substring(filter)
        };

        let reader = match self.disk_readers.get_mut(process) {
            Some(r) => r,
            None => return,
        };

        let current_total = match mode {
            StreamMode::Stdout => reader.line_count(LineSource::Stdout),
            StreamMode::Stderr => reader.line_count(LineSource::Stderr),
            StreamMode::Both => reader.line_count_both(),
        };

        if current_total <= scanned_up_to {
            return;
        }

        let new_matches = reader.scan_matching_lines_mode_from(&match_mode, mode, scanned_up_to);

        let fi = self.filtered_indices.get_mut(process).unwrap();
        fi.matching_lines.extend(new_matches);
        fi.scanned_up_to = current_total;
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

    pub fn failed_count(&self) -> usize {
        self.processes
            .iter()
            .filter(|p| p.state == crate::protocol::ProcessState::Failed)
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
            restart_count: None,
            max_restarts: None,
            restart_policy: None,
            watched: None,
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
        assert_eq!(buf.stdout_lines().count(), 1);
        assert_eq!(buf.stdout_lines().next().unwrap(), "hello world");
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

    #[test]
    fn test_output_buffer_counters() {
        let mut buf = OutputBuffer::new(5);
        assert_eq!(buf.len(), 0);
        assert_eq!(buf.stdout_count(), 0);
        assert_eq!(buf.stderr_count(), 0);
        assert!(buf.is_empty());

        buf.push(LineSource::Stdout, "a".into());
        buf.push(LineSource::Stderr, "b".into());
        buf.push(LineSource::Stdout, "c".into());
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.stdout_count(), 2);
        assert_eq!(buf.stderr_count(), 1);

        // Fill to capacity and trigger eviction
        buf.push(LineSource::Stdout, "d".into());
        buf.push(LineSource::Stderr, "e".into());
        assert_eq!(buf.len(), 5);
        assert_eq!(buf.stdout_count(), 3);
        assert_eq!(buf.stderr_count(), 2);

        // Push one more — evicts "a" (Stdout)
        buf.push(LineSource::Stderr, "f".into());
        assert_eq!(buf.len(), 5);
        assert_eq!(buf.stdout_count(), 2);
        assert_eq!(buf.stderr_count(), 3);
    }

    #[test]
    fn test_visible_lines_hot_buffer_only() {
        let mut app = App::new();
        // No disk readers, just hot buffer
        for i in 0..20 {
            app.push_output("web", Stream::Stdout, &format!("line {}", i));
        }
        app.update_processes(vec![make_process("web", ProcessState::Running)]);
        app.visible_height = 10;

        // Unpaused: should get the last 10 lines
        let lines = app.visible_lines("web", 10).unwrap();
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0].1, "line 10");
        assert_eq!(lines[9].1, "line 19");
    }

    #[test]
    fn test_visible_lines_paused_scrolled() {
        let mut app = App::new();
        for i in 0..50 {
            app.push_output("web", Stream::Stdout, &format!("line {}", i));
        }
        app.update_processes(vec![make_process("web", ProcessState::Running)]);
        app.visible_height = 10;
        app.paused = true;
        app.scroll_offsets.insert("web".into(), 20);

        let lines = app.visible_lines("web", 10).unwrap();
        assert_eq!(lines.len(), 10);
        // 50 total, scroll_offset=20, visible=10 → window [20..30)
        assert_eq!(lines[0].1, "line 20");
        assert_eq!(lines[9].1, "line 29");
    }

    #[test]
    fn test_visible_lines_with_disk_reader() {
        use crate::daemon::log_index::{IndexRecord, IndexWriter, idx_path_for};
        use crate::disk_log_reader::DiskLogReader;

        let dir = tempfile::tempdir().unwrap();

        // Create a disk log with 100 lines
        let log_path = dir.path().join("web.stdout");
        let idx_path = idx_path_for(&log_path);
        let mut log_content = String::new();
        let mut writer = IndexWriter::create(&idx_path, 0).unwrap();
        let mut offset = 0u64;
        for i in 0..100 {
            let line = format!("disk line {}", i);
            writer
                .append(IndexRecord {
                    byte_offset: offset,
                    seq: i,
                })
                .unwrap();
            log_content.push_str(&line);
            log_content.push('\n');
            offset += line.len() as u64 + 1;
        }
        writer.flush().unwrap();
        std::fs::write(&log_path, log_content).unwrap();

        let mut app = App::new();
        app.disk_readers.insert(
            "web".into(),
            DiskLogReader::new(dir.path().to_path_buf(), "web".into()),
        );

        // Push the last 10 lines into hot buffer (simulating live streaming)
        for i in 90..100 {
            app.push_output("web", Stream::Stdout, &format!("disk line {}", i));
        }

        app.update_processes(vec![make_process("web", ProcessState::Running)]);
        app.visible_height = 10;

        // Total should be 100 (disk is authoritative)
        assert_eq!(app.total_line_count("web"), 100);

        // Unpaused: last 10 lines from hot buffer
        let lines = app.visible_lines("web", 10).unwrap();
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0].1, "disk line 90");
        assert_eq!(lines[9].1, "disk line 99");

        // Scroll to top: should read from disk
        app.paused = true;
        app.scroll_offsets.insert("web".into(), 90);
        let lines = app.visible_lines("web", 10).unwrap();
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0].1, "disk line 0");
        assert_eq!(lines[9].1, "disk line 9");

        // Scroll to middle (spanning disk/hot boundary)
        // hot buffer has lines 90-99, disk boundary = 100 - 10 = 90
        // scroll_offset=5 → window_end=95, window_start=85
        // lines 85-89 from disk, lines 90-94 from hot
        app.scroll_offsets.insert("web".into(), 5);
        let lines = app.visible_lines("web", 10).unwrap();
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0].1, "disk line 85");
        assert_eq!(lines[4].1, "disk line 89");
        assert_eq!(lines[5].1, "disk line 90");
        assert_eq!(lines[9].1, "disk line 94");
    }

    #[test]
    fn test_visible_lines_with_filter_uses_filtered_index() {
        let mut app = App::new();
        app.push_output("web", Stream::Stdout, "hello");
        app.filter = Some("hello".into());

        // Without a disk reader, returns empty (no filtered index built)
        let lines = app.visible_lines("web", 10).unwrap();
        assert!(lines.is_empty());
    }

    #[test]
    fn test_hot_line_count_with_and_without_filter() {
        let mut app = App::new();
        app.push_output("web", Stream::Stdout, "hello world");
        app.push_output("web", Stream::Stdout, "goodbye world");
        app.push_output("web", Stream::Stdout, "hello again");

        // Unfiltered: O(1) count
        assert_eq!(app.hot_line_count("web", None), 3);

        // Filtered: O(n) scan
        assert_eq!(app.hot_line_count("web", Some("hello")), 2);
        assert_eq!(app.hot_line_count("web", Some("xyz")), 0);
    }
}
