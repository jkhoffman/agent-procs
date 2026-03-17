use crate::protocol::{Request, Response};

pub async fn execute(
    session: &str,
    command: &str,
    name: Option<String>,
    port: Option<u16>,
    proxy: bool,
) -> i32 {
    if proxy && let Some(code) = crate::cli::enable_proxy(session, None).await {
        return code;
    }

    let req = Request::Run {
        command: command.into(),
        name,
        cwd: None,
        env: None,
        port,
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
