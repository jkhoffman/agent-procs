use crate::protocol::Stream as ProtoStream;
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;

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
) {
    let mut lines = BufReader::new(reader).lines();
    let mut file = match tokio::fs::File::create(log_path).await {
        Ok(f) => f,
        Err(_) => return,
    };
    let mut bytes_written: u64 = 0;

    while let Ok(Some(line)) = lines.next_line().await {
        // Write to log file (with rotation check)
        let line_bytes = line.len() as u64 + 1; // +1 for newline
        if max_bytes > 0 && bytes_written + line_bytes > max_bytes {
            // Rotate: close current, rename to .1, start fresh
            drop(file);
            let rotated = log_path.with_extension(format!(
                "{}.1",
                log_path.extension().unwrap_or_default().to_string_lossy()
            ));
            let _ = tokio::fs::rename(log_path, &rotated).await;
            file = match tokio::fs::File::create(log_path).await {
                Ok(f) => f,
                Err(_) => return,
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
