use crate::protocol::{Request, Response};

pub async fn execute(session: &str, command: &str, name: Option<String>) -> i32 {
    let req = Request::Run {
        command: command.into(),
        name,
        cwd: None,
        env: None,
        port: None,
    };
    match crate::cli::request(session, &req, true).await {
        Ok(Response::RunOk { name, id, pid, .. }) => {
            println!("{} (id: {}, pid: {})", name, id, pid);
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
