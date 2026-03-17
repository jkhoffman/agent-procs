//! Background daemon: process lifecycle, output capture, and reverse proxy.
//!
//! Each session runs a single daemon process that listens on a Unix domain
//! socket.  The daemon manages child processes ([`process_manager`]),
//! captures their stdout/stderr to log files ([`log_writer`]), evaluates
//! wait conditions ([`wait_engine`]), and optionally runs a subdomain-based
//! reverse proxy ([`proxy`]).
//!
//! The daemon is spawned automatically by the CLI on first use (see
//! [`spawn`]) and exits when a `Shutdown` request is received.

pub mod actor;
pub mod log_index;
pub mod log_writer;
pub mod port_allocator;
pub mod process_manager;
pub mod proxy;
pub mod server;
pub mod spawn;
pub mod wait_engine;
