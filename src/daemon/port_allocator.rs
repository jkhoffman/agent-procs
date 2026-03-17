use crate::error::ProxyError;
use std::collections::HashSet;

const AUTO_PORT_MIN: u16 = 4000;
const AUTO_PORT_MAX: u16 = 4999;

impl Default for PortAllocator {
    fn default() -> Self {
        Self::new()
    }
}

pub struct PortAllocator {
    proxy_enabled: bool,
    next_auto_port: u16,
}

impl PortAllocator {
    pub fn new() -> Self {
        Self {
            proxy_enabled: false,
            next_auto_port: AUTO_PORT_MIN,
        }
    }

    pub fn enable_proxy(&mut self) {
        self.proxy_enabled = true;
    }

    pub fn is_proxy_enabled(&self) -> bool {
        self.proxy_enabled
    }

    pub fn auto_assign_port(&mut self, assigned: &HashSet<u16>) -> Result<u16, ProxyError> {
        let start = self.next_auto_port;
        let range_size = (AUTO_PORT_MAX - AUTO_PORT_MIN + 1) as usize;

        for i in 0..range_size {
            let candidate = AUTO_PORT_MIN
                + (((self.next_auto_port - AUTO_PORT_MIN) as usize + i) % range_size) as u16;
            if assigned.contains(&candidate) {
                continue;
            }
            // Bind-test: if we can bind, the port is free (listener drops immediately)
            if std::net::TcpListener::bind(("127.0.0.1", candidate)).is_ok() {
                self.next_auto_port = if candidate >= AUTO_PORT_MAX {
                    AUTO_PORT_MIN
                } else {
                    candidate + 1
                };
                return Ok(candidate);
            }
        }
        Err(ProxyError::NoFreeAutoPort {
            min: AUTO_PORT_MIN,
            max: AUTO_PORT_MAX,
            start,
        })
    }
}
