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

/// Current protocol version.  Bumped when breaking changes are introduced.
pub const PROTOCOL_VERSION: u32 = 1;

/// Typed error codes for [`Response::Error`].
///
/// Wire-compatible with the previous `i32` representation: serializes as
/// `1` (`General`) or `2` (`NotFound`).  Unknown codes from future versions
/// map to `General`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "i32", from = "i32")]
pub enum ErrorCode {
    General = 1,
    NotFound = 2,
}

impl ErrorCode {
    pub fn exit_code(self) -> i32 {
        self as i32
    }
}

impl From<i32> for ErrorCode {
    fn from(v: i32) -> Self {
        match v {
            2 => Self::NotFound,
            _ => Self::General,
        }
    }
}

impl From<ErrorCode> for i32 {
    fn from(c: ErrorCode) -> i32 {
        c as i32
    }
}

/// Restart behavior for supervised processes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RestartPolicy {
    pub mode: RestartMode,
    pub max_restarts: Option<u32>,
    pub restart_delay_ms: u64,
}

/// When a process should be automatically restarted.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartMode {
    Always,
    OnFailure,
    Never,
}

impl RestartMode {
    /// Parse a mode string (from CLI or config). Unknown values map to `Never`.
    pub fn parse(s: &str) -> Self {
        match s {
            "always" => Self::Always,
            "on-failure" => Self::OnFailure,
            _ => Self::Never,
        }
    }

    /// Whether this mode should trigger a restart given the exit code.
    pub fn should_restart(self, exit_code: Option<i32>) -> bool {
        match self {
            Self::Never => false,
            Self::Always => true,
            Self::OnFailure => exit_code != Some(0),
        }
    }
}

impl RestartPolicy {
    /// Build from CLI/config string arguments.
    pub fn from_args(mode: &str, max_restarts: Option<u32>, restart_delay: Option<u64>) -> Self {
        Self {
            mode: RestartMode::parse(mode),
            max_restarts,
            restart_delay_ms: restart_delay.unwrap_or(1000),
        }
    }
}

impl WatchConfig {
    /// Build from CLI/config path and ignore lists. Returns `None` if paths is empty.
    pub fn from_args(paths: Vec<String>, ignore: Vec<String>) -> Option<Self> {
        if paths.is_empty() {
            None
        } else {
            Some(Self {
                paths,
                ignore: if ignore.is_empty() {
                    None
                } else {
                    Some(ignore)
                },
            })
        }
    }
}

/// File-watch configuration for auto-restart on changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchConfig {
    pub paths: Vec<String>,
    #[serde(default)]
    pub ignore: Option<Vec<String>>,
}

/// Build the canonical URL for a managed process.
///
/// When `proxy_port` is `Some`, returns the subdomain-based proxy URL;
/// otherwise returns a direct `127.0.0.1` URL.
pub fn process_url(name: &str, port: u16, proxy_port: Option<u16>) -> String {
    match proxy_port {
        Some(pp) => format!("http://{}.localhost:{}", name, pp),
        None => format!("http://127.0.0.1:{}", port),
    }
}

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
        #[serde(default)]
        restart: Option<RestartPolicy>,
        #[serde(default)]
        watch: Option<WatchConfig>,
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
    Hello {
        version: u32,
    },
    /// Catch-all for unknown request types from future protocol versions.
    #[serde(other)]
    Unknown,
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
        code: ErrorCode,
        message: String,
    },
    Hello {
        version: u32,
    },
    /// Catch-all for unknown response types from future protocol versions.
    #[serde(other)]
    Unknown,
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
    #[serde(default)]
    pub restart_count: Option<u32>,
    #[serde(default)]
    pub max_restarts: Option<u32>,
    #[serde(default)]
    pub restart_policy: Option<String>,
    #[serde(default)]
    pub watched: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessState {
    Running,
    Exited,
    Failed,
    Unknown,
}

impl std::fmt::Display for ProcessState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Exited => write!(f, "exited"),
            Self::Failed => write!(f, "FAILED"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

impl<'de> serde::Deserialize<'de> for ProcessState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "running" => Self::Running,
            "exited" => Self::Exited,
            "failed" => Self::Failed,
            _ => Self::Unknown,
        })
    }
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
                restart: None,
                watch: None,
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
            Request::Hello {
                version: PROTOCOL_VERSION,
            },
            Request::Unknown,
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
                code: ErrorCode::General,
                message: "oops".into(),
            },
            Response::Hello {
                version: PROTOCOL_VERSION,
            },
            Response::Unknown,
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
            restart_count: None,
            max_restarts: None,
            restart_policy: None,
            watched: None,
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
            restart: None,
            watch: None,
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

        let hello = Request::Hello { version: 1 };
        let json = serde_json::to_string(&hello).unwrap();
        assert!(json.contains("\"type\":\"Hello\""));
    }

    #[test]
    fn test_unknown_request_deserialization() {
        // Unknown types should deserialize to Unknown
        let json = r#"{"type":"FutureRequestType","data":"something"}"#;
        let parsed: Request = serde_json::from_str(json).unwrap();
        assert_eq!(parsed, Request::Unknown);
    }

    #[test]
    fn test_unknown_response_deserialization() {
        let json = r#"{"type":"FutureResponseType","data":"something"}"#;
        let parsed: Response = serde_json::from_str(json).unwrap();
        assert_eq!(parsed, Response::Unknown);
    }

    #[test]
    fn test_error_code_wire_format() {
        let resp = Response::Error {
            code: ErrorCode::NotFound,
            message: "not found".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"code\":2"));

        // i32 codes from old clients deserialize correctly
        let json = r#"{"type":"Error","code":2,"message":"not found"}"#;
        let parsed: Response = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed,
            Response::Error {
                code: ErrorCode::NotFound,
                message: "not found".into(),
            }
        );

        // Unknown codes map to General
        let json = r#"{"type":"Error","code":99,"message":"future error"}"#;
        let parsed: Response = serde_json::from_str(json).unwrap();
        if let Response::Error { code, .. } = parsed {
            assert_eq!(code, ErrorCode::General);
        } else {
            panic!("expected Error");
        }
    }
}
