//! Domain-specific error types for `agent-procs`.
//!
//! Each module area has its own error enum so callers can match on
//! specific failure modes rather than parsing opaque strings.

use thiserror::Error;

/// Errors from configuration file parsing and dependency resolution.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("no agent-procs.yaml found")]
    NotFound,

    #[error("cannot get cwd: {0}")]
    Cwd(#[source] std::io::Error),

    #[error("cannot read config: {0}")]
    Read(#[source] std::io::Error),

    #[error("invalid config: {0}")]
    Parse(#[source] serde_yaml::Error),

    #[error("dependency cycle detected")]
    CycleDetected,

    #[error("unknown dependency: {from} depends on {to}")]
    UnknownDep { from: String, to: String },
}

/// Errors from the CLI client communicating with the daemon.
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("no daemon running for this session")]
    NoDaemon,

    #[error("failed to spawn daemon: {0}")]
    SpawnFailed(#[source] std::io::Error),

    #[error("failed to connect to daemon: {0}")]
    ConnectionFailed(#[source] std::io::Error),

    #[error("serialize error: {0}")]
    Serialize(#[source] serde_json::Error),

    #[error("write error: {0}")]
    Write(#[source] std::io::Error),

    #[error("flush error: {0}")]
    Flush(#[source] std::io::Error),

    #[error("read error: {0}")]
    Read(#[source] std::io::Error),

    #[error("parse error: {0}")]
    ParseResponse(#[source] serde_json::Error),
}

/// Errors from the reverse proxy subsystem.
#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("requested proxy port {port} is not available: {source}")]
    PortUnavailable { port: u16, source: std::io::Error },

    #[error("no free proxy port available in range {min}-{max}")]
    NoFreePort { min: u16, max: u16 },

    #[error("no free port available in range {min}-{max} (started at {start})")]
    NoFreeAutoPort { min: u16, max: u16, start: u16 },
}
