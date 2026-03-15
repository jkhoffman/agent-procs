use std::fs;
use std::path::Path;

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

pub struct IdCounter { next_id: u32 }

impl IdCounter {
    pub fn new() -> Self { Self { next_id: 1 } }
    pub fn next(&mut self) -> String {
        let id = format!("p{}", self.next_id);
        self.next_id += 1;
        id
    }
}
