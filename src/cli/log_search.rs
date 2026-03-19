//! Filter pipeline for CLI log search: `--grep`, `--since`, `--context`,
//! `--tail`, and `--json` output formatting.
//!
//! The pipeline runs in this order:
//! 1. `resolve_since()` — determine the starting line number
//! 2. `scan_matching_lines_mode_from()` — find matching line indices
//! 3. Take last `tail` matches (windowing)
//! 4. `expand_context()` — expand matches with N surrounding lines
//! 5. `read_result_lines()` — read actual text for output

use crate::disk_log_reader::{DiskLogReader, LineSource, MatchMode, StreamMode};
use serde::Serialize;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A set of search results for a single process.
pub struct SearchResult {
    pub process: String,
    pub lines: Vec<SearchLine>,
}

/// A single line in the search output.
pub struct SearchLine {
    pub line_number: usize,
    pub source: LineSource,
    pub text: String,
    pub is_context: bool,
}

/// Parameters controlling the search pipeline.
pub struct SearchParams {
    pub grep: Option<String>,
    pub regex: bool,
    pub since: Option<String>,
    pub tail: usize,
    pub context: Option<u32>,
    pub stderr: bool,
}

impl SearchParams {
    /// Compile the grep pattern into a `MatchMode`, or return `None` if no
    /// grep pattern was specified. Returns `Err` on invalid regex.
    pub fn match_mode(&self) -> Result<Option<MatchMode>, String> {
        match &self.grep {
            None => Ok(None),
            Some(pat) => {
                if self.regex {
                    let re = regex::Regex::new(pat)
                        .map_err(|e| format!("invalid regex '{}': {}", pat, e))?;
                    Ok(Some(MatchMode::Regex(re)))
                } else {
                    Ok(Some(MatchMode::Substring(pat.clone())))
                }
            }
        }
    }

    /// Which stream to search based on the `--stderr` flag.
    pub fn stream_mode(&self) -> StreamMode {
        if self.stderr {
            StreamMode::Stderr
        } else {
            StreamMode::Stdout
        }
    }
}

// ---------------------------------------------------------------------------
// Duration parsing
// ---------------------------------------------------------------------------

/// Parse a human-readable duration spec into seconds.
///
/// Accepted formats: `"30s"`, `"5m"`, `"2h"`. The numeric part must be a
/// positive integer.
pub fn parse_duration(spec: &str) -> Result<u64, String> {
    if spec.is_empty() {
        return Err("empty duration".to_string());
    }

    let (num_str, suffix) = spec.split_at(spec.len() - 1);
    let multiplier = match suffix {
        "s" => 1u64,
        "m" => 60,
        "h" => 3600,
        _ => {
            return Err(format!(
                "unknown duration suffix '{}' (expected s/m/h)",
                suffix
            ));
        }
    };

    let n: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid number in duration '{}'", spec))?;

    if n == 0 {
        return Err("duration must be positive".to_string());
    }

    Ok(n * multiplier)
}

// ---------------------------------------------------------------------------
// Since resolution
// ---------------------------------------------------------------------------

/// Resolve a `--since` value to a starting line number in the given stream.
///
/// - `"start"` — line 0 (beginning of all history)
/// - `"restart"` — line after the last restart marker
/// - Duration specs (`"30s"`, `"5m"`, `"2h"`) — estimate position based on
///   uptime; clamps to incarnation boundary when available
fn resolve_since(
    reader: &mut DiskLogReader,
    since: &str,
    stream: StreamMode,
    uptime_secs: Option<u64>,
) -> Result<usize, String> {
    match since {
        "start" => Ok(0),
        "restart" => resolve_since_restart(reader, stream),
        other => {
            let secs = parse_duration(other)?;
            Ok(resolve_since_duration(reader, stream, secs, uptime_secs))
        }
    }
}

/// Resolve `--since restart`: find the line after the last restart marker.
///
/// For stderr mode, cross-reference the stdout marker's sequence number to
/// find the corresponding stderr position.
fn resolve_since_restart(reader: &mut DiskLogReader, stream: StreamMode) -> Result<usize, String> {
    let marker = reader.find_last_restart_marker();
    match marker {
        None => {
            // No restart marker found — start from beginning
            Ok(0)
        }
        Some(stdout_line) => {
            let after_marker = stdout_line + 1;
            match stream {
                StreamMode::Stdout | StreamMode::Both => Ok(after_marker),
                StreamMode::Stderr => {
                    // Cross-stream resolution: find stderr position corresponding
                    // to the stdout marker's sequence number.
                    reader
                        .find_stderr_position_by_stdout_seq(stdout_line)
                        .map_err(|e| format!("cross-stream resolution failed: {}", e))
                }
            }
        }
    }
}

/// Resolve a duration-based `--since` to an estimated line position.
///
/// Strategy: use `uptime_secs` and total line count to estimate lines/sec,
/// then seek backward from the end.
fn resolve_since_duration(
    reader: &mut DiskLogReader,
    stream: StreamMode,
    duration_secs: u64,
    uptime_secs: Option<u64>,
) -> usize {
    let total = match stream {
        StreamMode::Stdout => reader.line_count(LineSource::Stdout),
        StreamMode::Stderr => reader.line_count(LineSource::Stderr),
        StreamMode::Both => reader.line_count_both(),
    };

    if total == 0 {
        return 0;
    }

    let uptime = match uptime_secs {
        Some(u) if u > 0 => u,
        _ => {
            eprintln!(
                "warning: process uptime unavailable; --since duration will start from incarnation beginning"
            );
            return incarnation_start(reader, stream);
        }
    };

    // Estimate: lines_per_sec = total / uptime
    // lines_in_window = lines_per_sec * duration_secs
    let lines_in_window = if uptime > 0 {
        ((total as u64) * duration_secs / uptime) as usize
    } else {
        total
    };

    // Clamp: don't go before incarnation start
    let incarnation = incarnation_start(reader, stream);
    let estimated_start = total.saturating_sub(lines_in_window);
    estimated_start.max(incarnation)
}

/// Find the incarnation start (line after last restart marker, or 0).
fn incarnation_start(reader: &mut DiskLogReader, stream: StreamMode) -> usize {
    resolve_since_restart(reader, stream).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Context expansion
// ---------------------------------------------------------------------------

/// Expand a set of match line indices into ranges including context lines.
///
/// Returns `(line_number, is_context)` pairs sorted by line number, with
/// overlapping ranges merged.
pub fn expand_context(matches: &[usize], context: u32, total_lines: usize) -> Vec<(usize, bool)> {
    if matches.is_empty() || total_lines == 0 {
        return Vec::new();
    }

    let ctx = context as usize;
    let match_set: std::collections::HashSet<usize> = matches.iter().copied().collect();

    // Build merged ranges
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for &m in matches {
        let start = m.saturating_sub(ctx);
        let end = (m + ctx + 1).min(total_lines);
        if let Some(last) = ranges.last_mut()
            && start <= last.1
        {
            // Overlapping or adjacent — merge
            last.1 = last.1.max(end);
            continue;
        }
        ranges.push((start, end));
    }

    // Expand ranges into individual lines
    let mut result = Vec::new();
    for (start, end) in ranges {
        for line in start..end {
            let is_context = !match_set.contains(&line);
            result.push((line, is_context));
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Main pipeline
// ---------------------------------------------------------------------------

/// Run the full search pipeline for a single process.
///
/// Pipeline:
/// 1. Resolve `--since` to starting line
/// 2. Scan for matching lines (or all lines if no grep)
/// 3. Take last `tail` matches
/// 4. Expand context around matches
/// 5. Read actual line text
pub fn search_process(
    reader: &mut DiskLogReader,
    process: &str,
    params: &SearchParams,
    uptime_secs: Option<u64>,
) -> Result<SearchResult, String> {
    let stream = params.stream_mode();
    let match_mode = params.match_mode()?;

    // 1. Resolve starting line
    let start_line = match &params.since {
        Some(since) => resolve_since(reader, since, stream, uptime_secs)?,
        None => 0,
    };

    // Total lines in the stream
    let total = match stream {
        StreamMode::Stdout => reader.line_count(LineSource::Stdout),
        StreamMode::Stderr => reader.line_count(LineSource::Stderr),
        StreamMode::Both => reader.line_count_both(),
    };

    // 2. Find matching line indices
    let all_matches = match &match_mode {
        Some(mode) => reader.scan_matching_lines_mode_from(mode, stream, start_line),
        None => {
            // No grep — all lines from start_line are "matches"
            (start_line..total).collect()
        }
    };

    // 3. Window: take last `tail` matches
    let windowed: Vec<usize> = if params.tail > 0 && all_matches.len() > params.tail {
        all_matches[all_matches.len() - params.tail..].to_vec()
    } else {
        all_matches
    };

    // 4. Context expansion
    let expanded: Vec<(usize, bool)> = match params.context {
        Some(ctx) if !windowed.is_empty() => expand_context(&windowed, ctx, total),
        _ => windowed.iter().map(|&ln| (ln, false)).collect(),
    };

    // 5. Read result lines
    let source = match stream {
        StreamMode::Stderr => LineSource::Stderr,
        StreamMode::Stdout | StreamMode::Both => LineSource::Stdout,
    };

    let line_numbers: Vec<usize> = expanded.iter().map(|&(ln, _)| ln).collect();
    let read_lines = reader.read_scattered_lines(stream, &line_numbers);

    let mut lines = Vec::with_capacity(expanded.len());
    for (i, &(line_number, is_context)) in expanded.iter().enumerate() {
        let (line_source, text) = if i < read_lines.len() {
            read_lines[i].clone()
        } else {
            (source, String::new())
        };
        lines.push(SearchLine {
            line_number,
            source: line_source,
            text,
            is_context,
        });
    }

    Ok(SearchResult {
        process: process.to_string(),
        lines,
    })
}

// ---------------------------------------------------------------------------
// Output formatting
// ---------------------------------------------------------------------------

/// A single JSONL output record.
#[derive(Serialize)]
struct JsonOutputLine {
    process: String,
    line_number: usize,
    stream: String,
    text: String,
    #[serde(skip_serializing_if = "is_false")]
    context: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

fn source_name(source: LineSource) -> &'static str {
    match source {
        LineSource::Stdout => "stdout",
        LineSource::Stderr => "stderr",
    }
}

/// Print search results in text or JSON format.
///
/// Text mode: prints lines with `--` separators between non-adjacent groups.
/// JSON mode: JSONL with process, `line_number`, stream, text, context fields.
pub fn print_results(results: &[SearchResult], json: bool, _context: Option<u32>) {
    if json {
        print_results_json(results);
    } else {
        print_results_text(results);
    }
}

fn print_results_json(results: &[SearchResult]) {
    for result in results {
        for line in &result.lines {
            let record = JsonOutputLine {
                process: result.process.clone(),
                line_number: line.line_number,
                stream: source_name(line.source).to_string(),
                text: line.text.clone(),
                context: line.is_context,
            };
            if let Ok(json) = serde_json::to_string(&record) {
                println!("{}", json);
            }
        }
    }
}

fn print_results_text(results: &[SearchResult]) {
    let multi_process = results.len() > 1;

    for (ri, result) in results.iter().enumerate() {
        if result.lines.is_empty() {
            continue;
        }

        if multi_process && ri > 0 {
            // Blank line between processes
            println!();
        }

        let mut prev_line_number: Option<usize> = None;
        for line in &result.lines {
            // Print separator between non-adjacent groups
            if let Some(prev) = prev_line_number
                && line.line_number > prev + 1
            {
                println!("--");
            }

            if multi_process {
                println!("[{}] {}", result.process, line.text);
            } else {
                println!("{}", line.text);
            }

            prev_line_number = Some(line.line_number);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_duration tests -----------------------------------------------

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("30s").unwrap(), 30);
        assert_eq!(parse_duration("1s").unwrap(), 1);
        assert_eq!(parse_duration("120s").unwrap(), 120);
    }

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("5m").unwrap(), 300);
        assert_eq!(parse_duration("1m").unwrap(), 60);
    }

    #[test]
    fn test_parse_duration_hours() {
        assert_eq!(parse_duration("2h").unwrap(), 7200);
        assert_eq!(parse_duration("1h").unwrap(), 3600);
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("30").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("30d").is_err());
        assert!(parse_duration("0s").is_err());
        assert!(parse_duration("-5s").is_err());
    }

    // -- expand_context tests -----------------------------------------------

    #[test]
    fn test_expand_context_no_context() {
        // context=0 should return just the matches
        let matches = vec![3, 7, 12];
        let result = expand_context(&matches, 0, 20);
        assert_eq!(result, vec![(3, false), (7, false), (12, false)]);
    }

    #[test]
    fn test_expand_context_with_context() {
        // context=1 around line 5 in a 10-line file
        let matches = vec![5];
        let result = expand_context(&matches, 1, 10);
        assert_eq!(result, vec![(4, true), (5, false), (6, true)]);
    }

    #[test]
    fn test_expand_context_merge_overlapping() {
        // Two matches close together: context should merge
        let matches = vec![3, 5];
        let result = expand_context(&matches, 1, 10);
        // 3-1=2..3+2=5 and 5-1=4..5+2=7 → merged: 2..7
        assert_eq!(
            result,
            vec![(2, true), (3, false), (4, true), (5, false), (6, true),]
        );
    }

    #[test]
    fn test_expand_context_clamps_to_bounds() {
        // Match at line 0 with context=3 in a 5-line file
        let matches = vec![0];
        let result = expand_context(&matches, 3, 5);
        assert_eq!(result, vec![(0, false), (1, true), (2, true), (3, true),]);

        // Match at last line
        let matches = vec![4];
        let result = expand_context(&matches, 3, 5);
        assert_eq!(result, vec![(1, true), (2, true), (3, true), (4, false),]);
    }

    #[test]
    fn test_expand_context_empty_matches() {
        let result = expand_context(&[], 2, 10);
        assert!(result.is_empty());
    }

    #[test]
    fn test_expand_context_zero_total() {
        let result = expand_context(&[0], 2, 0);
        assert!(result.is_empty());
    }
}
