use crate::protocol::{Request, Response};

pub async fn execute(
    session: &str, target: &str, until: Option<String>,
    regex: bool, exit: bool, timeout: Option<u64>,
) -> i32 {
    let req = Request::Wait {
        target: target.into(), until, regex, exit,
        timeout_secs: timeout,
    };
    match crate::cli::request(session, &req, false).await {
        Ok(Response::WaitMatch { line }) => { println!("{}", line); 0 }
        Ok(Response::WaitExited { exit_code }) => { println!("exited with code {}", exit_code); 0 }
        Ok(Response::WaitTimeout) => { eprintln!("timeout"); 1 }
        Ok(Response::Error { code, message }) => { eprintln!("error: {}", message); code }
        Ok(_) => { eprintln!("unexpected response"); 1 }
        Err(e) => { eprintln!("error: {}", e); 1 }
    }
}
