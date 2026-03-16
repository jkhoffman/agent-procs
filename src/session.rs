//! Daemon health checks and unique ID generation.
//!
//! [`is_daemon_alive`] probes a PID file to determine whether a session's
//! daemon process is still running.  [`IdCounter`] hands out sequential
//! process identifiers (`p1`, `p2`, …) within a daemon's lifetime.

use std::fs;
use std::path::Path;

#[must_use]
pub fn is_daemon_alive(pid_path: &Path) -> bool {
    let content = match fs::read_to_string(pid_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let pid: i32 = match content.trim().parse() {
        Ok(p) => p,
        Err(_) => return false,
    };
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

pub struct IdCounter {
    next_id: u32,
}

impl Default for IdCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl IdCounter {
    pub fn new() -> Self {
        Self { next_id: 1 }
    }
    pub fn next_id(&mut self) -> String {
        let id = format!("p{}", self.next_id);
        self.next_id += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_counter_sequential() {
        let mut counter = IdCounter::new();
        assert_eq!(counter.next_id(), "p1");
        assert_eq!(counter.next_id(), "p2");
        assert_eq!(counter.next_id(), "p3");
    }

    #[test]
    fn test_id_counter_default() {
        let mut counter = IdCounter::default();
        assert_eq!(counter.next_id(), "p1");
    }
}
