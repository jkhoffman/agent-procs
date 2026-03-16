use crate::config::{load_config, resolve_session};

pub async fn execute(cli_session: Option<&str>) -> i32 {
    let session = match cli_session {
        Some(s) => s.to_string(),
        None => {
            let config_session = load_config(None).ok().and_then(|(_, c)| c.session);
            resolve_session(None, config_session.as_deref()).to_string()
        }
    };

    crate::cli::stop::execute_all(&session).await
}
