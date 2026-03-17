use crate::protocol::Stream as ProtoStream;
use std::path::Path;
use std::sync::LazyLock;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;

/// Regex matching ANSI escape sequences (CSI, OSC, and simple two-byte escapes).
static ANSI_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\x1b\[[0-9;]*[A-Za-z]|\x1b\].*?(\x07|\x1b\\)|\x1b[()][A-B0-2]").unwrap()
});

/// Strip ANSI escape sequences from a string.
fn strip_ansi(s: &str) -> String {
    ANSI_RE.replace_all(s, "").into_owned()
}

/// Default number of rotated log files to keep.
pub const DEFAULT_MAX_ROTATED_FILES: u32 = 5;

/// A line of output from a child process.
#[derive(Debug, Clone)]
pub struct OutputLine {
    pub process: String,
    pub stream: ProtoStream,
    pub line: String,
}

/// Reads lines from a child's stdout or stderr, writes each line to a log file,
/// and broadcasts it to subscribers (wait engine, --follow clients).
pub async fn capture_output<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    log_path: &Path,
    process_name: &str,
    stream: ProtoStream,
    tx: broadcast::Sender<OutputLine>,
    max_bytes: u64,
    max_rotated_files: u32,
) {
    let mut lines = BufReader::new(reader).lines();
    let mut file = match tokio::fs::File::create(log_path).await {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(path = %log_path.display(), process = %process_name, error = %e, "cannot create log file");
            return;
        }
    };
    let mut bytes_written: u64 = 0;

    while let Ok(Some(raw_line)) = lines.next_line().await {
        let line = strip_ansi(&raw_line);
        // Write to log file (with rotation check)
        let line_bytes = line.len() as u64 + 1; // +1 for newline
        if max_bytes > 0 && bytes_written + line_bytes > max_bytes {
            // Rotate: cascade .N-1 → .N, then current → .1, delete excess
            drop(file);
            rotate_log_files(log_path, max_rotated_files).await;
            file = match tokio::fs::File::create(log_path).await {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!(path = %log_path.display(), process = %process_name, error = %e, "cannot recreate log file after rotation");
                    return;
                }
            };
            bytes_written = 0;
        }

        let _ = file.write_all(line.as_bytes()).await;
        let _ = file.write_all(b"\n").await;
        let _ = file.flush().await;
        bytes_written += line_bytes;

        // Broadcast to wait engine / followers
        let _ = tx.send(OutputLine {
            process: process_name.to_string(),
            stream,
            line,
        });
    }
}

/// Cascade-rotate log files: shift .N-1 → .N down to .1 → .2,
/// then rename current → .1, and delete files beyond `max_rotated_files`.
async fn rotate_log_files(log_path: &Path, max_rotated_files: u32) {
    let ext = log_path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    // Shift existing rotated files: .N-1 → .N (starting from highest to avoid overwriting)
    for i in (1..max_rotated_files).rev() {
        let from = log_path.with_extension(format!("{}.{}", ext, i));
        let to = log_path.with_extension(format!("{}.{}", ext, i + 1));
        let _ = tokio::fs::rename(&from, &to).await;
    }

    // Rename current → .1
    let rotated_1 = log_path.with_extension(format!("{}.1", ext));
    let _ = tokio::fs::rename(log_path, &rotated_1).await;

    // Delete excess files beyond max_rotated_files
    let excess = log_path.with_extension(format!("{}.{}", ext, max_rotated_files + 1));
    let _ = tokio::fs::remove_file(&excess).await;
}
