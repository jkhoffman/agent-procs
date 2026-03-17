//! Random-access line reading from disk-backed logs using sidecar index files.
//!
//! [`DiskLogReader`] provides windowed access to log history, allowing the TUI
//! to scroll through the entire output history without loading everything into
//! memory.

use crate::daemon::log_index::{IndexReader, idx_path_for};
use crate::tui::app::LineSource;
use std::io::{self, BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Info about a single log segment (one log file + its index).
#[derive(Clone)]
struct Segment {
    log_path: PathBuf,
    idx_path: PathBuf,
    line_count: usize,
}

/// Cached segment list with a short TTL to avoid repeated filesystem probes.
struct SegmentCache {
    stdout: Vec<Segment>,
    stderr: Vec<Segment>,
    refreshed_at: Instant,
}

/// How long to cache segment enumerations before re-probing the filesystem.
const SEGMENT_CACHE_TTL_MS: u128 = 500;

/// Provides random-access line reading from disk log files using the sidecar
/// `.idx` index.  Handles rotated files (`.1`, `.2`, ...) transparently.
pub struct DiskLogReader {
    log_dir: PathBuf,
    process: String,
    cache: Option<SegmentCache>,
}

impl DiskLogReader {
    pub fn new(log_dir: PathBuf, process: String) -> Self {
        Self {
            log_dir,
            process,
            cache: None,
        }
    }

    /// Get cached segments for a source, refreshing if stale.
    fn segments(&mut self, source: LineSource) -> &[Segment] {
        let stale = self
            .cache
            .as_ref()
            .is_none_or(|c| c.refreshed_at.elapsed().as_millis() > SEGMENT_CACHE_TTL_MS);
        if stale {
            let stdout = Self::discover_segments(&self.log_dir, &self.process, LineSource::Stdout);
            let stderr = Self::discover_segments(&self.log_dir, &self.process, LineSource::Stderr);
            self.cache = Some(SegmentCache {
                stdout,
                stderr,
                refreshed_at: Instant::now(),
            });
        }
        let cache = self.cache.as_ref().unwrap();
        match source {
            LineSource::Stdout => &cache.stdout,
            LineSource::Stderr => &cache.stderr,
        }
    }

    /// Total line count for a stream across current + rotated files.
    pub fn line_count(&mut self, source: LineSource) -> usize {
        self.segments(source).iter().map(|s| s.line_count).sum()
    }

    /// Total line count for merged both-streams view.
    pub fn line_count_both(&mut self) -> usize {
        // Single cache refresh covers both streams.
        let stdout: usize = self
            .segments(LineSource::Stdout)
            .iter()
            .map(|s| s.line_count)
            .sum();
        let stderr: usize = self
            .segments(LineSource::Stderr)
            .iter()
            .map(|s| s.line_count)
            .sum();
        stdout + stderr
    }

    /// Read lines `[start..start+count)` for a single stream, spanning
    /// rotated files oldest-to-newest.
    pub fn read_lines(
        &mut self,
        source: LineSource,
        start: usize,
        count: usize,
    ) -> io::Result<Vec<String>> {
        let segments = self.segments(source).to_vec();
        read_lines_from_segments(&segments, start, count)
    }

    /// Read interleaved lines for "Both" mode, merge-sorted by sequence number.
    pub fn read_interleaved(
        &mut self,
        start: usize,
        count: usize,
    ) -> io::Result<Vec<(LineSource, String)>> {
        let merged = self.build_merged_index()?;
        let end = (start + count).min(merged.len());
        if start >= end {
            return Ok(Vec::new());
        }

        let window = &merged[start..end];

        // Batch reads: group by source, read each stream's lines in one pass,
        // then assemble in merged order.
        let mut stdout_needed: Vec<(usize, usize)> = Vec::new(); // (result_idx, stream_line)
        let mut stderr_needed: Vec<(usize, usize)> = Vec::new();
        for (i, &(source, stream_line)) in window.iter().enumerate() {
            match source {
                LineSource::Stdout => stdout_needed.push((i, stream_line)),
                LineSource::Stderr => stderr_needed.push((i, stream_line)),
            }
        }

        let mut result: Vec<(LineSource, String)> =
            vec![(LineSource::Stdout, String::new()); end - start];

        // Read stdout lines in batch
        for &(result_idx, stream_line) in &stdout_needed {
            let line = Self::read_single_line_from(self.segments(LineSource::Stdout), stream_line)?;
            result[result_idx] = (LineSource::Stdout, line);
        }
        // Read stderr lines in batch
        for &(result_idx, stream_line) in &stderr_needed {
            let line = Self::read_single_line_from(self.segments(LineSource::Stderr), stream_line)?;
            result[result_idx] = (LineSource::Stderr, line);
        }

        Ok(result)
    }

    /// Build merged index: all lines from both streams sorted by sequence number.
    fn build_merged_index(&mut self) -> io::Result<Vec<(LineSource, usize)>> {
        let mut entries: Vec<(u64, LineSource, usize)> = Vec::new();

        for source in [LineSource::Stdout, LineSource::Stderr] {
            let segments = self.segments(source).to_vec();
            let mut line_offset = 0;
            for seg in &segments {
                if let Ok(Some(mut reader)) = IndexReader::open(&seg.idx_path) {
                    let records = reader.read_range(0, seg.line_count)?;
                    for (i, rec) in records.iter().enumerate() {
                        entries.push((rec.seq, source, line_offset + i));
                    }
                } else {
                    for i in 0..seg.line_count {
                        entries.push((u64::MAX, source, line_offset + i));
                    }
                }
                line_offset += seg.line_count;
            }
        }

        entries.sort_by_key(|&(seq, _, _)| seq);
        Ok(entries
            .into_iter()
            .map(|(_, src, line)| (src, line))
            .collect())
    }

    /// Read a single line by absolute position within pre-fetched segments.
    fn read_single_line_from(segments: &[Segment], absolute_line: usize) -> io::Result<String> {
        let mut cumulative = 0;
        for seg in segments {
            if absolute_line < cumulative + seg.line_count {
                let line_in_seg = absolute_line - cumulative;
                let lines = read_lines_from_segment(&seg.log_path, &seg.idx_path, line_in_seg, 1)?;
                return lines.into_iter().next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::UnexpectedEof, "line not found in segment")
                });
            }
            cumulative += seg.line_count;
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "line {} out of range (total: {})",
                absolute_line, cumulative
            ),
        ))
    }

    /// Probe the filesystem for log segments of a given stream, ordered oldest first.
    fn discover_segments(log_dir: &Path, process: &str, source: LineSource) -> Vec<Segment> {
        let stream = match source {
            LineSource::Stdout => "stdout",
            LineSource::Stderr => "stderr",
        };
        let base = log_dir.join(format!("{}.{}", process, stream));

        let mut rotated: Vec<(u32, Segment)> = Vec::new();
        for n in 1u32.. {
            let log_path = base.with_extension(format!("{}.{}", stream, n));
            if !log_path.exists() {
                break;
            }
            let idx_path = idx_path_for(&log_path);
            let line_count = idx_line_count(&idx_path)
                .unwrap_or_else(|| count_lines_in_file(&log_path).unwrap_or(0));
            rotated.push((
                n,
                Segment {
                    log_path,
                    idx_path,
                    line_count,
                },
            ));
        }

        // Sort by N descending so highest-N (oldest) comes first
        rotated.sort_by(|a, b| b.0.cmp(&a.0));
        let mut segments: Vec<Segment> = rotated.into_iter().map(|(_, s)| s).collect();

        if base.exists() {
            let idx_path = idx_path_for(&base);
            let line_count = idx_line_count(&idx_path)
                .unwrap_or_else(|| count_lines_in_file(&base).unwrap_or(0));
            segments.push(Segment {
                log_path: base,
                idx_path,
                line_count,
            });
        }

        segments
    }
}

/// Read lines from a span of segments by absolute line range.
fn read_lines_from_segments(
    segments: &[Segment],
    start: usize,
    count: usize,
) -> io::Result<Vec<String>> {
    let mut result = Vec::with_capacity(count);
    let mut cumulative = 0;

    for seg in segments {
        let seg_end = cumulative + seg.line_count;
        let window_end = start + count;

        if seg_end <= start || cumulative >= window_end {
            cumulative = seg_end;
            continue;
        }

        let read_start = start.max(cumulative) - cumulative;
        let read_end = window_end.min(seg_end) - cumulative;
        let lines = read_lines_from_segment(
            &seg.log_path,
            &seg.idx_path,
            read_start,
            read_end - read_start,
        )?;
        result.extend(lines);

        cumulative = seg_end;
    }
    Ok(result)
}

/// Get line count from an index file's metadata (no content read).
fn idx_line_count(idx_path: &Path) -> Option<usize> {
    IndexReader::line_count_from_metadata(idx_path).ok()
}

/// Fallback: count lines by scanning the log file.
fn count_lines_in_file(path: &Path) -> io::Result<usize> {
    let file = std::fs::File::open(path)?;
    Ok(BufReader::new(file).lines().count())
}

/// Read `count` lines starting at `start` from a single segment.
/// Uses the index for seeking when available, falls back to sequential scan.
fn read_lines_from_segment(
    log_path: &Path,
    idx_path: &Path,
    start: usize,
    count: usize,
) -> io::Result<Vec<String>> {
    if count == 0 {
        return Ok(Vec::new());
    }

    // Try indexed read
    if let Ok(Some(mut idx_reader)) = IndexReader::open(idx_path) {
        let records = idx_reader.read_range(start, count)?;
        if records.is_empty() {
            return Ok(Vec::new());
        }
        let file = std::fs::File::open(log_path)?;
        let mut reader = BufReader::new(file);
        // Seek to first record and read sequentially (records are contiguous)
        reader.seek(SeekFrom::Start(records[0].byte_offset))?;
        let mut result = Vec::with_capacity(records.len());
        for _ in 0..records.len() {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            if line.ends_with('\n') {
                line.pop();
            }
            result.push(line);
        }
        return Ok(result);
    }

    // Fallback: sequential scan
    let file = std::fs::File::open(log_path)?;
    let lines: Vec<String> = BufReader::new(file)
        .lines()
        .skip(start)
        .take(count)
        .collect::<io::Result<Vec<_>>>()?;
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::log_index::{IndexRecord, IndexWriter};

    /// Helper: create a log file with the given lines and a matching index.
    fn create_log_with_index(dir: &Path, filename: &str, lines: &[&str], seq_base: u64) {
        let log_path = dir.join(filename);
        let idx_path = idx_path_for(&log_path);

        let mut log_content = String::new();
        let mut writer = IndexWriter::create(&idx_path, seq_base).unwrap();
        let mut offset: u64 = 0;

        for (i, line) in lines.iter().enumerate() {
            writer
                .append(IndexRecord {
                    byte_offset: offset,
                    seq: seq_base + i as u64,
                })
                .unwrap();
            log_content.push_str(line);
            log_content.push('\n');
            offset += line.len() as u64 + 1;
        }
        writer.flush().unwrap();
        std::fs::write(&log_path, log_content).unwrap();
    }

    #[test]
    fn test_line_count_single_file() {
        let dir = tempfile::tempdir().unwrap();
        create_log_with_index(dir.path(), "test.stdout", &["line1", "line2", "line3"], 0);

        let mut reader = DiskLogReader::new(dir.path().to_path_buf(), "test".to_string());
        assert_eq!(reader.line_count(LineSource::Stdout), 3);
        assert_eq!(reader.line_count(LineSource::Stderr), 0);
    }

    #[test]
    fn test_read_lines_single_file() {
        let dir = tempfile::tempdir().unwrap();
        create_log_with_index(
            dir.path(),
            "test.stdout",
            &["aaa", "bbb", "ccc", "ddd", "eee"],
            0,
        );

        let mut reader = DiskLogReader::new(dir.path().to_path_buf(), "test".to_string());
        let lines = reader.read_lines(LineSource::Stdout, 1, 3).unwrap();
        assert_eq!(lines, vec!["bbb", "ccc", "ddd"]);
    }

    #[test]
    fn test_read_lines_across_rotated() {
        let dir = tempfile::tempdir().unwrap();
        // Rotated file .2 (oldest)
        create_log_with_index(dir.path(), "test.stdout.2", &["old1", "old2"], 0);
        // Rotated file .1
        create_log_with_index(dir.path(), "test.stdout.1", &["mid1", "mid2"], 2);
        // Current file (newest)
        create_log_with_index(dir.path(), "test.stdout", &["new1", "new2"], 4);

        let mut reader = DiskLogReader::new(dir.path().to_path_buf(), "test".to_string());
        assert_eq!(reader.line_count(LineSource::Stdout), 6);

        // Read across segments
        let lines = reader.read_lines(LineSource::Stdout, 1, 4).unwrap();
        assert_eq!(lines, vec!["old2", "mid1", "mid2", "new1"]);
    }

    #[test]
    fn test_read_interleaved() {
        let dir = tempfile::tempdir().unwrap();
        // Stdout: seq 0, 2, 4
        create_log_with_index(dir.path(), "test.stdout", &["out0", "out2", "out4"], 0);
        // Manually set seq numbers to 0, 2, 4
        let idx_path = idx_path_for(&dir.path().join("test.stdout"));
        let mut writer = IndexWriter::create(&idx_path, 0).unwrap();
        writer
            .append(IndexRecord {
                byte_offset: 0,
                seq: 0,
            })
            .unwrap();
        writer
            .append(IndexRecord {
                byte_offset: 5,
                seq: 2,
            })
            .unwrap();
        writer
            .append(IndexRecord {
                byte_offset: 10,
                seq: 4,
            })
            .unwrap();
        writer.flush().unwrap();

        // Stderr: seq 1, 3
        let stderr_content = "err1\nerr3\n";
        std::fs::write(dir.path().join("test.stderr"), stderr_content).unwrap();
        let idx_path = idx_path_for(&dir.path().join("test.stderr"));
        let mut writer = IndexWriter::create(&idx_path, 1).unwrap();
        writer
            .append(IndexRecord {
                byte_offset: 0,
                seq: 1,
            })
            .unwrap();
        writer
            .append(IndexRecord {
                byte_offset: 5,
                seq: 3,
            })
            .unwrap();
        writer.flush().unwrap();

        let mut reader = DiskLogReader::new(dir.path().to_path_buf(), "test".to_string());
        assert_eq!(reader.line_count_both(), 5);

        let interleaved = reader.read_interleaved(0, 5).unwrap();
        assert_eq!(interleaved.len(), 5);
        assert_eq!(interleaved[0], (LineSource::Stdout, "out0".to_string()));
        assert_eq!(interleaved[1], (LineSource::Stderr, "err1".to_string()));
        assert_eq!(interleaved[2], (LineSource::Stdout, "out2".to_string()));
        assert_eq!(interleaved[3], (LineSource::Stderr, "err3".to_string()));
        assert_eq!(interleaved[4], (LineSource::Stdout, "out4".to_string()));
    }

    #[test]
    fn test_fallback_no_index() {
        let dir = tempfile::tempdir().unwrap();
        // Log file without index
        std::fs::write(dir.path().join("test.stdout"), "aaa\nbbb\nccc\n").unwrap();

        let mut reader = DiskLogReader::new(dir.path().to_path_buf(), "test".to_string());
        assert_eq!(reader.line_count(LineSource::Stdout), 3);

        let lines = reader.read_lines(LineSource::Stdout, 1, 2).unwrap();
        assert_eq!(lines, vec!["bbb", "ccc"]);
    }

    #[test]
    fn test_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut reader = DiskLogReader::new(dir.path().to_path_buf(), "test".to_string());
        assert_eq!(reader.line_count(LineSource::Stdout), 0);
        assert_eq!(reader.line_count_both(), 0);
    }
}
