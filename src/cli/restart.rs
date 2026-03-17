use crate::protocol::{Request, Response};

pub async fn execute(session: &str, target: &str) -> i32 {
    let req = Request::Restart {
        target: target.into(),
    };
    crate::cli::request_and_handle(session, &req, false, |resp| match resp {
        Response::RunOk { name, id, pid, .. } => {
            println!("restarted {} (id: {}, pid: {})", name, id, pid);
            Some(0)
        }
        _ => None,
    })
    .await
}
