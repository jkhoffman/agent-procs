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

/// State for writing lines to a log file with index and broadcast.
struct LogWriteState {
    file: tokio::fs::File,
    idx_writer: IndexWriter,
    bytes_written: u64,
    lines_since_idx_flush: u32,
}

impl LogWriteState {
    #[allow(clippy::too_many_arguments)]
    async fn write_line(
        &mut self,
        line: &str,
        log_path: &Path,
        process_name: &str,
        stream: ProtoStream,
        tx: &broadcast::Sender<OutputLine>,
        max_bytes: u64,
        max_rotated_files: u32,
        seq: &AtomicU64,
    ) {
        let line_bytes = line.len() as u64 + 1;
        if max_bytes > 0 && self.bytes_written + line_bytes > max_bytes {
            let _ = self.idx_writer.flush();
            // Rotate: drop file handles, rotate, reopen
            let placeholder = tokio::fs::File::create("/dev/null").await.unwrap();
            let old_file = std::mem::replace(&mut self.file, placeholder);
            drop(old_file);

            rotate_log_files(log_path, max_rotated_files).await;
            self.file = match tokio::fs::File::create(log_path).await {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!(path = %log_path.display(), process = %process_name, error = %e, "cannot recreate log file after rotation");
                    return;
                }
            };
            let idx_path = idx_path_for(log_path);
            let new_seq_base = seq.load(Ordering::Relaxed);
            self.idx_writer = match IndexWriter::create(&idx_path, new_seq_base) {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!(path = %idx_path.display(), error = %e, "cannot recreate index file after rotation");
                    return;
                }
            };
            self.bytes_written = 0;
            self.lines_since_idx_flush = 0;
        }

        let byte_offset = self.bytes_written;
        let line_seq = seq.fetch_add(1, Ordering::Relaxed);
        let _ = self.idx_writer.append(IndexRecord {
            byte_offset,
            seq: line_seq,
        });

        let _ = self.file.write_all(line.as_bytes()).await;
        let _ = self.file.write_all(b"\n").await;
        let _ = self.file.flush().await;
        self.bytes_written += line_bytes;

        self.lines_since_idx_flush += 1;
        if self.lines_since_idx_flush >= 64 {
            let _ = self.idx_writer.flush();
            self.lines_since_idx_flush = 0;
        }

        let _ = tx.send(OutputLine {
            process: process_name.to_string(),
            stream,
            line: line.to_string(),
        });
    }
}

/// Reads lines from a child's stdout or stderr, writes each line to a log file
/// with a sidecar `.idx` index, and broadcasts it to subscribers.
///
/// Also accepts a supervisor channel for synthetic log lines (e.g. restart
/// annotations). After the pipe hits EOF, continues draining the supervisor
/// channel until it is closed.
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
    mut sup_rx: tokio::sync::mpsc::Receiver<String>,
) {
    let mut lines = BufReader::new(reader).lines();
    let file = match tokio::fs::File::create(log_path).await {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(path = %log_path.display(), process = %process_name, error = %e, "cannot create log file");
            return;
        }
    };

    let idx_path = idx_path_for(log_path);
    let seq_base = seq.load(Ordering::Relaxed);
    let idx_writer = match IndexWriter::create(&idx_path, seq_base) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(path = %idx_path.display(), error = %e, "cannot create index file");
            return;
        }
    };

    let mut state = LogWriteState {
        file,
        idx_writer,
        bytes_written: 0,
        lines_since_idx_flush: 0,
    };

    // Main loop: select between pipe reads and supervisor channel
    loop {
        let line = tokio::select! {
            result = lines.next_line() => {
                match result {
                    Ok(Some(line)) => line,
                    _ => break, // pipe EOF — fall through to drain supervisor channel
                }
            }
            Some(sup_line) = sup_rx.recv() => sup_line,
        };

        state
            .write_line(
                &line,
                log_path,
                process_name,
                stream,
                &tx,
                max_bytes,
                max_rotated_files,
                &seq,
            )
            .await;
    }

    // After pipe EOF, drain remaining supervisor lines
    while let Some(sup_line) = sup_rx.recv().await {
        state
            .write_line(
                &sup_line,
                log_path,
                process_name,
                stream,
                &tx,
                max_bytes,
                max_rotated_files,
                &seq,
            )
            .await;
    }

    // Flush remaining buffered index entries
    let _ = state.idx_writer.flush();
}

/// Rotate log files if the path exists. Used by `respawn_in_place()`.
pub async fn rotate_if_exists(log_path: &Path) {
    if log_path.exists() {
        rotate_log_files(log_path, DEFAULT_MAX_ROTATED_FILES).await;
    }
}

/// Cascade-rotate log files and their `.idx` companions:
/// shift .N-1 → .N down to .1 → .2, then rename current → .1,
/// and delete files beyond `max_rotated_files`.
pub(crate) async fn rotate_log_files(log_path: &Path, max_rotated_files: u32) {
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

    /// Create a dummy supervisor channel (sender dropped immediately).
    fn dummy_sup_rx() -> tokio::sync::mpsc::Receiver<String> {
        let (_tx, rx) = tokio::sync::mpsc::channel::<String>(1);
        rx
    }

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
            dummy_sup_rx(),
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
            dummy_sup_rx(),
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
            dummy_sup_rx(),
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
            dummy_sup_rx(),
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

    #[tokio::test]
    async fn test_capture_output_supervisor_channel() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("test.stdout");
        let log_path_clone = log_path.clone();
        let (tx, _rx) = broadcast::channel(16);
        let seq = Arc::new(AtomicU64::new(0));
        let (sup_tx, sup_rx) = tokio::sync::mpsc::channel::<String>(16);

        // Use a duplex to control timing: pipe data is consumed first,
        // then supervisor line is drained after pipe EOF.
        let (client, server) = tokio::io::duplex(1024);
        let mut writer = client;
        use tokio::io::AsyncWriteExt as _;
        let write_task = tokio::spawn(async move {
            writer.write_all(b"line 0\nline 1\n").await.unwrap();
            writer.shutdown().await.unwrap();
            // Wait for pipe EOF to be processed before sending supervisor line
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            sup_tx
                .send("[agent-procs] Restarted".to_string())
                .await
                .unwrap();
            drop(sup_tx);
        });

        capture_output(
            server,
            &log_path_clone,
            "test",
            ProtoStream::Stdout,
            tx,
            1024 * 1024,
            5,
            seq.clone(),
            sup_rx,
        )
        .await;

        write_task.await.unwrap();

        let content = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "line 0");
        assert_eq!(lines[1], "line 1");
        assert_eq!(lines[2], "[agent-procs] Restarted");

        // Index should have 3 entries
        let idx_path = idx_path_for(&log_path);
        let idx = IndexReader::open(&idx_path).unwrap().unwrap();
        assert_eq!(idx.line_count(), 3);

        // Seq counter should be at 3
        assert_eq!(seq.load(Ordering::Relaxed), 3);
    }
}
