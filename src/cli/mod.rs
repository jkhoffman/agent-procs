//! CLI client: connecting to the daemon and sending requests.
//!
//! Each subcommand module (`run`, `stop`, `status`, …) builds a
//! [`Request`](crate::protocol::Request), sends it via [`request`], and
//! interprets the [`Response`](crate::protocol::Response).
//!
//! [`connect`] handles auto-spawning the daemon on first use.
//! [`stream_responses`] supports streaming commands like `logs --follow`.

pub mod down;
pub mod logs;
pub mod restart;
pub mod run;
pub mod session_cmd;
pub mod status;
pub mod stop;
pub mod up;
pub mod wait;

use crate::error::ClientError;
use crate::paths;
use crate::protocol::{Request, Response, Stream as ProtoStream};
use crate::session;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

pub async fn connect(session: &str, auto_spawn: bool) -> Result<UnixStream, ClientError> {
    let socket = paths::socket_path(session);
    let pid = paths::pid_path(session);

    if !session::is_daemon_alive(&pid) {
        if auto_spawn {
            crate::daemon::spawn::spawn_daemon(session).map_err(ClientError::SpawnFailed)?;
        } else {
            return Err(ClientError::NoDaemon);
        }
    }

    UnixStream::connect(&socket)
        .await
        .map_err(ClientError::ConnectionFailed)
}

/// Serialize a request and write it to the socket.
async fn send_request(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    req: &Request,
) -> Result<(), ClientError> {
    let mut json = serde_json::to_string(req).map_err(ClientError::Serialize)?;
    json.push('\n');
    writer
        .write_all(json.as_bytes())
        .await
        .map_err(ClientError::Write)?;
    writer.flush().await.map_err(ClientError::Flush)
}

pub async fn request(
    session: &str,
    req: &Request,
    auto_spawn: bool,
) -> Result<Response, ClientError> {
    let stream = connect(session, auto_spawn).await?;
    let (reader, mut writer) = stream.into_split();

    send_request(&mut writer, req).await?;

    let mut lines = BufReader::new(reader);
    let mut line = String::new();
    lines
        .read_line(&mut line)
        .await
        .map_err(ClientError::Read)?;

    serde_json::from_str(&line).map_err(ClientError::ParseResponse)
}

/// Send an `EnableProxy` request to the daemon. Returns `Some(exit_code)` on error, None on success.
pub async fn enable_proxy(session: &str, proxy_port: Option<u16>) -> Option<i32> {
    let req = Request::EnableProxy { proxy_port };
    match request(session, &req, true).await {
        Ok(Response::Ok { message }) => {
            eprintln!("{}", message);
            None
        }
        Ok(Response::Error { code, message }) => {
            eprintln!("error enabling proxy: {}", message);
            Some(code)
        }
        Err(e) => {
            eprintln!("error enabling proxy: {}", e);
            Some(1)
        }
        _ => None,
    }
}

/// Send a request and read streaming responses until `LogEnd` or error.
/// Calls `on_line` for each `LogLine` received. Returns the terminal response.
pub async fn stream_responses(
    session: &str,
    req: &Request,
    auto_spawn: bool,
    mut on_line: impl FnMut(&str, ProtoStream, &str),
) -> Result<Response, ClientError> {
    let stream = connect(session, auto_spawn).await?;
    let (reader, mut writer) = stream.into_split();

    send_request(&mut writer, req).await?;

    let mut lines = BufReader::new(reader);
    loop {
        let mut line = String::new();
        let n = lines
            .read_line(&mut line)
            .await
            .map_err(ClientError::Read)?;
        if n == 0 {
            return Ok(Response::LogEnd);
        } // EOF

        let resp: Response = serde_json::from_str(&line).map_err(ClientError::ParseResponse)?;
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
