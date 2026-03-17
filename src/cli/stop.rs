use crate::protocol::{Request, Response};

pub async fn execute(session: &str, target: &str) -> i32 {
    let req = Request::Stop {
        target: target.into(),
    };
    crate::cli::request_and_handle(session, &req, false, |resp| match resp {
        Response::Ok { message } => {
            println!("{}", message);
            Some(0)
        }
        _ => None,
    })
    .await
}

pub async fn execute_all(session: &str) -> i32 {
    let req = Request::StopAll;
    crate::cli::request_and_handle(session, &req, false, |resp| match resp {
        Response::Ok { message } => {
            println!("{}", message);
            Some(0)
        }
        _ => None,
    })
    .await
}
