use crate::config::{load_config, resolve_session};
use crate::protocol::Request;

pub async fn execute(cli_session: Option<&str>) -> i32 {
    let session = match cli_session {
        Some(s) => s.to_string(),
        None => {
            let config_session = load_config(None).ok().and_then(|(_, c)| c.session);
            resolve_session(None, config_session.as_deref()).to_string()
        }
    };

    let code = crate::cli::stop::execute_all(&session).await;
    if code == 0 {
        // Shut down the daemon — it will auto-spawn on next use
        let _ = crate::cli::request(&session, &Request::Shutdown, false).await;
    }
    code
}
