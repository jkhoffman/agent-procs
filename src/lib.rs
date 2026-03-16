#![deny(unsafe_code)]

//! `agent-procs` — a concurrent process runner for AI agents.
//!
//! Processes run in a background daemon and persist across CLI invocations.
//! Communication happens over a per-session Unix domain socket using a
//! JSON-lines protocol (see [`protocol`]).
//!
//! # Modules
//!
//! - [`config`] — YAML configuration file parsing and dependency ordering
//! - [`protocol`] — Request/Response types for daemon–CLI communication
//! - [`daemon`] — Background daemon: process lifecycle, logging, reverse proxy
//! - [`cli`] — CLI client: connecting to the daemon and sending requests
//! - [`tui`] — Terminal UI for real-time process monitoring
//! - [`paths`] — Socket, PID, and log directory resolution
//! - [`session`] — Daemon health checks and ID generation

pub mod cli;
pub mod config;
pub mod daemon;
pub mod error;
pub mod paths;
pub mod protocol;
pub mod session;
pub mod tui;
