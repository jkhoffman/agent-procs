//! Wire protocol for daemon–CLI communication.
//!
//! All messages are serialized as single-line JSON (newline-delimited).
//! The client sends a [`Request`] and reads back one or more [`Response`]s
//! (streaming commands like `Logs --follow` produce multiple responses).
//!
//! Both types use `#[serde(tag = "type")]` so the JSON includes a `"type"`
//! discriminant field for easy interop with non-Rust clients.
//!
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Maximum allowed size (in bytes) for a single JSON message line.
/// Provides defense-in-depth against runaway reads on the Unix socket.
pub const MAX_MESSAGE_SIZE: usize = 1024 * 1024; // 1 MiB

/// A client-to-daemon request, serialized as tagged JSON.
///
/// # Examples
///
/// ```
/// use agent_procs::protocol::Request;
///
/// let req = Request::Status;
/// let json = serde_json::to_string(&req).unwrap();
/// assert!(json.contains(r#""type":"Status""#));
///
/// let parsed: Request = serde_json::from_str(&json).unwrap();
/// assert_eq!(parsed, req);
/// ```
#[must_use]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    Run {
        command: String,
        name: Option<String>,
        cwd: Option<String>,
        #[serde(default)]
        env: Option<HashMap<String, String>>,
        #[serde(default)]
        port: Option<u16>,
    },
    Stop {
        target: String,
    },
    StopAll,
    Restart {
        target: String,
    },
    Status,
    Logs {
        target: Option<String>,
        tail: usize,
        follow: bool,
        stderr: bool,
        all: bool,
        timeout_secs: Option<u64>,
        #[serde(default)]
        lines: Option<usize>,
    },
    Wait {
        target: String,
        until: Option<String>,
        regex: bool,
        exit: bool,
        timeout_secs: Option<u64>,
    },
    Shutdown,
    EnableProxy {
        #[serde(default)]
        proxy_port: Option<u16>,
    },
}

/// A daemon-to-client response, serialized as tagged JSON.
///
/// # Examples
///
/// ```
/// use agent_procs::protocol::Response;
///
/// let resp = Response::Ok { message: "done".into() };
/// let json = serde_json::to_string(&resp).unwrap();
/// let parsed: Response = serde_json::from_str(&json).unwrap();
/// assert_eq!(parsed, resp);
/// ```
#[must_use]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    Ok {
        message: String,
    },
    RunOk {
        name: String,
        id: String,
        pid: u32,
        #[serde(default)]
        port: Option<u16>,
        #[serde(default)]
        url: Option<String>,
    },
    Status {
        processes: Vec<ProcessInfo>,
    },
    LogLine {
        process: String,
        stream: Stream,
        line: String,
    },
    LogEnd,
    WaitMatch {
        line: String,
    },
    WaitExited {
        exit_code: Option<i32>,
    },
    WaitTimeout,
    Error {
        code: i32,
        message: String,
    },
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub name: String,
    pub id: String,
    pub pid: u32,
    pub state: ProcessState,
    pub exit_code: Option<i32>,
    pub uptime_secs: Option<u64>,
    pub command: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessState {
    Running,
    Exited,
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    Stdout,
    Stderr,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serde_roundtrip() {
        let requests = vec![
            Request::Run {
                command: "echo hi".into(),
                name: Some("test".into()),
                cwd: None,
                env: None,
                port: None,
            },
            Request::Stop {
                target: "test".into(),
            },
            Request::StopAll,
            Request::Restart {
                target: "test".into(),
            },
            Request::Status,
            Request::Logs {
                target: Some("test".into()),
                tail: 10,
                follow: true,
                stderr: false,
                all: false,
                timeout_secs: Some(30),
                lines: None,
            },
            Request::Wait {
                target: "test".into(),
                until: Some("ready".into()),
                regex: false,
                exit: false,
                timeout_secs: Some(60),
            },
            Request::Shutdown,
            Request::EnableProxy {
                proxy_port: Some(8080),
            },
        ];

        for req in &requests {
            let json = serde_json::to_string(req).unwrap();
            let parsed: Request = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, req);
        }
    }

    #[test]
    fn test_response_serde_roundtrip() {
        let responses = vec![
            Response::Ok {
                message: "done".into(),
            },
            Response::RunOk {
                name: "web".into(),
                id: "p1".into(),
                pid: 1234,
                port: Some(3000),
                url: Some("http://127.0.0.1:3000".into()),
            },
            Response::Status { processes: vec![] },
            Response::LogLine {
                process: "web".into(),
                stream: Stream::Stdout,
                line: "hello".into(),
            },
            Response::LogEnd,
            Response::WaitMatch {
                line: "ready".into(),
            },
            Response::WaitExited { exit_code: Some(0) },
            Response::WaitTimeout,
            Response::Error {
                code: 1,
                message: "oops".into(),
            },
        ];

        for resp in &responses {
            let json = serde_json::to_string(resp).unwrap();
            let parsed: Response = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, resp);
        }
    }

    #[test]
    fn test_process_info_serde_roundtrip() {
        let info = ProcessInfo {
            name: "api".into(),
            id: "p1".into(),
            pid: 42,
            state: ProcessState::Running,
            exit_code: None,
            uptime_secs: Some(120),
            command: "cargo run".into(),
            port: Some(8080),
            url: Some("http://127.0.0.1:8080".into()),
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: ProcessInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, info);
    }

    #[test]
    fn test_request_json_format() {
        let run = Request::Run {
            command: "ls".into(),
            name: None,
            cwd: None,
            env: None,
            port: None,
        };
        let json = serde_json::to_string(&run).unwrap();
        assert!(json.contains("\"type\":\"Run\""));

        let stop = Request::Stop { target: "x".into() };
        let json = serde_json::to_string(&stop).unwrap();
        assert!(json.contains("\"type\":\"Stop\""));

        let status = Request::Status;
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"type\":\"Status\""));

        let shutdown = Request::Shutdown;
        let json = serde_json::to_string(&shutdown).unwrap();
        assert!(json.contains("\"type\":\"Shutdown\""));

        let stop_all = Request::StopAll;
        let json = serde_json::to_string(&stop_all).unwrap();
        assert!(json.contains("\"type\":\"StopAll\""));

        let restart = Request::Restart { target: "x".into() };
        let json = serde_json::to_string(&restart).unwrap();
        assert!(json.contains("\"type\":\"Restart\""));

        let enable_proxy = Request::EnableProxy { proxy_port: None };
        let json = serde_json::to_string(&enable_proxy).unwrap();
        assert!(json.contains("\"type\":\"EnableProxy\""));
    }
}
