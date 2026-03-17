use crate::daemon::log_index::{IndexRecord, IndexWriter, idx_path_for};
use crate::protocol::Stream as ProtoStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;

/// Default number of rotated log files to keep.
pub const DEFAULT_MAX_ROTATED_FILES: u32 = 5;

/// A line of output from a child process.
#[derive(Debug, Clone)]
pub struct OutputLine {
    pub process: String,
    pub stream: ProtoStream,
    pub line: String,
}

/// Reads lines from a child's stdout or stderr, writes each line to a log file
/// with a sidecar `.idx` index, and broadcasts it to subscribers.
#[allow(clippy::too_many_arguments)]
pub async fn capture_output<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    log_path: &Path,
    process_name: &str,
    stream: ProtoStream,
    tx: broadcast::Sender<OutputLine>,
    max_bytes: u64,
    max_rotated_files: u32,
    seq: Arc<AtomicU64>,
) {
    let mut lines = BufReader::new(reader).lines();
    let mut file = match tokio::fs::File::create(log_path).await {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(path = %log_path.display(), process = %process_name, error = %e, "cannot create log file");
            return;
        }
    };

    let idx_path = idx_path_for(log_path);
    let seq_base = seq.load(Ordering::Relaxed);
    let mut idx_writer = match IndexWriter::create(&idx_path, seq_base) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(path = %idx_path.display(), error = %e, "cannot create index file");
            return;
        }
    };

    let mut bytes_written: u64 = 0;
    let mut lines_since_idx_flush: u32 = 0;

    while let Ok(Some(line)) = lines.next_line().await {
        // Write to log file (with rotation check)
        let line_bytes = line.len() as u64 + 1; // +1 for newline
        if max_bytes > 0 && bytes_written + line_bytes > max_bytes {
            // Flush index before rotation
            let _ = idx_writer.flush();

            // Rotate: cascade .N-1 → .N, then current → .1, delete excess
            drop(file);
            drop(idx_writer);
            rotate_log_files(log_path, max_rotated_files).await;
            file = match tokio::fs::File::create(log_path).await {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!(path = %log_path.display(), process = %process_name, error = %e, "cannot recreate log file after rotation");
                    return;
                }
            };
            let new_seq_base = seq.load(Ordering::Relaxed);
            idx_writer = match IndexWriter::create(&idx_path, new_seq_base) {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!(path = %idx_path.display(), error = %e, "cannot recreate index file after rotation");
                    return;
                }
            };
            bytes_written = 0;
            lines_since_idx_flush = 0;
        }

        // Record byte offset before writing the line
        let byte_offset = bytes_written;
        let line_seq = seq.fetch_add(1, Ordering::Relaxed);

        // Write index record (buffered; flushed periodically below)
        let _ = idx_writer.append(IndexRecord {
            byte_offset,
            seq: line_seq,
        });

        let _ = file.write_all(line.as_bytes()).await;
        let _ = file.write_all(b"\n").await;
        let _ = file.flush().await;
        bytes_written += line_bytes;

        // Flush index periodically to reduce syscalls while keeping
        // the sidecar reasonably up-to-date for TUI reads.
        lines_since_idx_flush += 1;
        if lines_since_idx_flush >= 64 {
            let _ = idx_writer.flush();
            lines_since_idx_flush = 0;
        }

        // Broadcast to wait engine / followers
        let _ = tx.send(OutputLine {
            process: process_name.to_string(),
            stream,
            line,
        });
    }
    // Flush remaining buffered index entries
    let _ = idx_writer.flush();
}

/// Cascade-rotate log files and their `.idx` companions:
/// shift .N-1 → .N down to .1 → .2, then rename current → .1,
/// and delete files beyond `max_rotated_files`.
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
        // Also rename idx companion
        let idx_from = idx_path_for(&from);
        let idx_to = idx_path_for(&to);
        let _ = tokio::fs::rename(&idx_from, &idx_to).await;
    }

    // Rename current → .1
    let rotated_1 = log_path.with_extension(format!("{}.1", ext));
    let _ = tokio::fs::rename(log_path, &rotated_1).await;
    // Rename current idx → .1.idx
    let idx_current = idx_path_for(log_path);
    let idx_rotated_1 = idx_path_for(&rotated_1);
    let _ = tokio::fs::rename(&idx_current, &idx_rotated_1).await;

    // Delete excess files beyond max_rotated_files
    let excess = log_path.with_extension(format!("{}.{}", ext, max_rotated_files + 1));
    let _ = tokio::fs::remove_file(&excess).await;
    let idx_excess = idx_path_for(&excess);
    let _ = tokio::fs::remove_file(&idx_excess).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::log_index::IndexReader;
    use std::sync::atomic::AtomicU64;

    #[tokio::test]
    async fn test_capture_output_creates_index() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("test.stdout");
        let (tx, _rx) = broadcast::channel(16);
        let seq = Arc::new(AtomicU64::new(0));

        // Feed 5 lines through capture_output
        let input = b"line 0\nline 1\nline 2\nline 3\nline 4\n";
        let reader = &input[..];
        capture_output(
            reader,
            &log_path,
            "test",
            ProtoStream::Stdout,
            tx,
            1024 * 1024,
            5,
            seq.clone(),
        )
        .await;

        // Log file should exist with all lines
        let content = std::fs::read_to_string(&log_path).unwrap();
        assert_eq!(content.lines().count(), 5);

        // Index file should exist alongside
        let idx_path = idx_path_for(&log_path);
        let mut idx = IndexReader::open(&idx_path).unwrap().unwrap();
        assert_eq!(idx.line_count(), 5);

        // First record: byte_offset=0, seq=0
        let r0 = idx.read_record(0).unwrap();
        assert_eq!(r0.byte_offset, 0);
        assert_eq!(r0.seq, 0);

        // Second record: byte_offset=7 ("line 0\n" = 7 bytes), seq=1
        let r1 = idx.read_record(1).unwrap();
        assert_eq!(r1.byte_offset, 7);
        assert_eq!(r1.seq, 1);

        // Seq counter should have advanced to 5
        assert_eq!(seq.load(Ordering::Relaxed), 5);
    }

    #[tokio::test]
    async fn test_capture_output_shared_seq_counter() {
        let dir = tempfile::tempdir().unwrap();
        let stdout_path = dir.path().join("test.stdout");
        let stderr_path = dir.path().join("test.stderr");
        let (tx, _rx) = broadcast::channel(16);
        let seq = Arc::new(AtomicU64::new(0));

        // Run stdout capture
        let stdout_input = b"out1\nout2\n";
        capture_output(
            &stdout_input[..],
            &stdout_path,
            "test",
            ProtoStream::Stdout,
            tx.clone(),
            1024 * 1024,
            5,
            seq.clone(),
        )
        .await;

        // seq should be at 2 now
        assert_eq!(seq.load(Ordering::Relaxed), 2);

        // Run stderr capture (continues from seq=2)
        let stderr_input = b"err1\nerr2\n";
        capture_output(
            &stderr_input[..],
            &stderr_path,
            "test",
            ProtoStream::Stderr,
            tx,
            1024 * 1024,
            5,
            seq.clone(),
        )
        .await;

        // seq should be at 4
        assert_eq!(seq.load(Ordering::Relaxed), 4);

        // Verify stderr index has seq 2 and 3
        let idx_path = idx_path_for(&stderr_path);
        let mut idx = IndexReader::open(&idx_path).unwrap().unwrap();
        assert_eq!(idx.read_record(0).unwrap().seq, 2);
        assert_eq!(idx.read_record(1).unwrap().seq, 3);
    }

    #[tokio::test]
    async fn test_capture_output_rotation_creates_idx_companions() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("test.stdout");
        let (tx, _rx) = broadcast::channel(64);
        let seq = Arc::new(AtomicU64::new(0));

        // max_bytes=50 will trigger rotation after a few lines
        // "line NN\n" = 8 bytes, so ~6 lines per file
        let mut input = String::new();
        for i in 0..20 {
            use std::fmt::Write;
            let _ = writeln!(input, "line {:02}", i);
        }

        capture_output(
            input.as_bytes(),
            &log_path,
            "test",
            ProtoStream::Stdout,
            tx,
            50,
            3,
            seq,
        )
        .await;

        // Current log file and idx should exist
        assert!(log_path.exists());
        assert!(idx_path_for(&log_path).exists());

        // At least one rotated file should exist
        let rotated = log_path.with_extension("stdout.1");
        assert!(
            rotated.exists(),
            "rotated log .1 should exist after rotation"
        );
        assert!(
            idx_path_for(&rotated).exists(),
            "rotated idx .1 should exist after rotation"
        );
    }
}
