use crate::protocol::{Request, Response};

pub async fn execute(session: &str, target: &str) -> i32 {
    let req = Request::Restart { target: target.into() };
    match crate::cli::request(session, &req, false).await {
        Ok(Response::RunOk { name, id, pid }) => { println!("restarted {} (id: {}, pid: {})", name, id, pid); 0 }
        Ok(Response::Error { code, message }) => { eprintln!("error: {}", message); code }
        _ => 1,
    }
}
