use crate::protocol::{Request, Response};

pub async fn execute(
    session: &str,
    command: &str,
    name: Option<String>,
    port: Option<u16>,
    proxy: bool,
) -> i32 {
    if proxy {
        if let Some(code) = crate::cli::enable_proxy(session, None).await {
            return code;
        }
    }

    let req = Request::Run {
        command: command.into(),
        name,
        cwd: None,
        env: None,
        port,
    };
    match crate::cli::request(session, &req, true).await {
        Ok(Response::RunOk { name, id, pid, url, .. }) => {
            match url {
                Some(u) => println!("{} (id: {}, pid: {}, {})", name, id, pid, u),
                None => println!("{} (id: {}, pid: {})", name, id, pid),
            }
            0
        }
        Ok(Response::Error { code, message }) => {
            eprintln!("error: {}", message);
            code
        }
        Ok(_) => {
            eprintln!("unexpected response");
            1
        }
        Err(e) => {
            eprintln!("error: {}", e);
            1
        }
    }
}
