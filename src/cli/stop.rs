use crate::protocol::{Request, Response};

pub async fn execute(session: &str, target: &str) -> i32 {
    let req = Request::Stop {
        target: target.into(),
    };
    match crate::cli::request(session, &req, false).await {
        Ok(Response::Ok { message }) => {
            println!("{}", message);
            0
        }
        Ok(Response::Error { code, message }) => {
            eprintln!("error: {}", message);
            code
        }
        _ => 1,
    }
}

pub async fn execute_all(session: &str) -> i32 {
    let req = Request::StopAll;
    match crate::cli::request(session, &req, false).await {
        Ok(Response::Ok { message }) => {
            println!("{}", message);
            0
        }
        Ok(Response::Error { code, message }) => {
            eprintln!("error: {}", message);
            code
        }
        _ => 1,
    }
}
