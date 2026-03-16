use std::path::Path;
use tokio::net::UnixListener;

pub async fn run(_session: &str, socket_path: &Path) {
    let listener = UnixListener::bind(socket_path).expect("failed to bind socket");
    loop {
        match listener.accept().await {
            Ok((_stream, _addr)) => {} // Implemented in Task 7
            Err(_) => break,
        }
    }
}
