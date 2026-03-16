pub mod down;
pub mod logs;
pub mod restart;
pub mod run;
pub mod session_cmd;
pub mod status;
pub mod stop;
pub mod up;
pub mod wait;

use crate::paths;
use crate::protocol::{Request, Response, Stream as ProtoStream};
use crate::session;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

pub async fn connect(session: &str, auto_spawn: bool) -> Result<UnixStream, String> {
    let socket = paths::socket_path(session);
    let pid = paths::pid_path(session);

    if !session::is_daemon_alive(&pid) {
        if auto_spawn {
            crate::daemon::spawn::spawn_daemon(session)
                .map_err(|e| format!("failed to spawn daemon: {}", e))?;
        } else {
            return Err("no daemon running for this session".into());
        }
    }

    UnixStream::connect(&socket)
        .await
        .map_err(|e| format!("failed to connect to daemon: {}", e))
}

pub async fn request(session: &str, req: &Request, auto_spawn: bool) -> Result<Response, String> {
    let stream = connect(session, auto_spawn).await?;
    let (reader, mut writer) = stream.into_split();

    let mut json = serde_json::to_string(req).unwrap();
    json.push('\n');
    writer
        .write_all(json.as_bytes())
        .await
        .map_err(|e| format!("write error: {}", e))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("flush error: {}", e))?;

    let mut lines = BufReader::new(reader);
    let mut line = String::new();
    lines
        .read_line(&mut line)
        .await
        .map_err(|e| format!("read error: {}", e))?;

    serde_json::from_str(&line).map_err(|e| format!("parse error: {}", e))
}

/// Send a request and read streaming responses until LogEnd or error.
/// Calls `on_line` for each LogLine received. Returns the terminal response.
pub async fn stream_responses(
    session: &str,
    req: &Request,
    auto_spawn: bool,
    mut on_line: impl FnMut(&str, ProtoStream, &str),
) -> Result<Response, String> {
    let stream = connect(session, auto_spawn).await?;
    let (reader, mut writer) = stream.into_split();

    let mut json = serde_json::to_string(req).unwrap();
    json.push('\n');
    writer
        .write_all(json.as_bytes())
        .await
        .map_err(|e| format!("write error: {}", e))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("flush error: {}", e))?;

    let mut lines = BufReader::new(reader);
    loop {
        let mut line = String::new();
        let n = lines
            .read_line(&mut line)
            .await
            .map_err(|e| format!("read error: {}", e))?;
        if n == 0 {
            return Ok(Response::LogEnd);
        } // EOF

        let resp: Response =
            serde_json::from_str(&line).map_err(|e| format!("parse error: {}", e))?;
        match resp {
            Response::LogLine {
                ref process,
                stream,
                ref line,
            } => {
                on_line(process, stream, line);
            }
            Response::LogEnd | Response::Error { .. } => return Ok(resp),
            other => return Ok(other), // unexpected
        }
    }
}
