use crate::protocol::{Request, Response, RestartPolicy, WatchConfig};

#[allow(clippy::too_many_arguments)]
pub async fn execute(
    session: &str,
    command: &str,
    name: Option<String>,
    port: Option<u16>,
    proxy: bool,
    autorestart: Option<String>,
    max_restarts: Option<u32>,
    restart_delay: Option<u64>,
    watch: Vec<String>,
    watch_ignore: Vec<String>,
) -> i32 {
    if proxy && let Some(code) = crate::cli::enable_proxy(session, None).await {
        return code;
    }

    let restart = autorestart.map(|m| RestartPolicy::from_args(&m, max_restarts, restart_delay));
    let watch_config = WatchConfig::from_args(watch, watch_ignore);

    let req = Request::Run {
        command: command.into(),
        name,
        cwd: None,
        env: None,
        port,
        restart,
        watch: watch_config,
    };
    crate::cli::request_and_handle(session, &req, true, |resp| match resp {
        Response::RunOk {
            name, id, pid, url, ..
        } => {
            match url {
                Some(u) => println!("{} (id: {}, pid: {}, {})", name, id, pid, u),
                None => println!("{} (id: {}, pid: {})", name, id, pid),
            }
            Some(0)
        }
        _ => None,
    })
    .await
}
