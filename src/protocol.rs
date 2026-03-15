use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    Run { command: String, name: Option<String>, cwd: Option<String> },
    Stop { target: String },
    StopAll,
    Restart { target: String },
    Status,
    Logs { target: Option<String>, tail: usize, follow: bool, stderr: bool, all: bool, timeout_secs: Option<u64> },
    Wait { target: String, until: Option<String>, regex: bool, exit: bool, timeout_secs: Option<u64> },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    Ok { message: String },
    RunOk { name: String, id: String, pid: u32 },
    Status { processes: Vec<ProcessInfo> },
    LogLine { process: String, stream: Stream, line: String },
    LogEnd,
    WaitMatch { line: String },
    WaitExited { exit_code: i32 },
    WaitTimeout,
    Error { code: i32, message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub name: String,
    pub id: String,
    pub pid: u32,
    pub state: ProcessState,
    pub exit_code: Option<i32>,
    pub uptime_secs: Option<u64>,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessState { Running, Exited }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream { Stdout, Stderr }
