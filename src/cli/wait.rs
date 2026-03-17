use crate::protocol::{Request, Response};

pub async fn execute(
    session: &str,
    target: &str,
    until: Option<String>,
    regex: bool,
    exit: bool,
    timeout: Option<u64>,
) -> i32 {
    let req = Request::Wait {
        target: target.into(),
        until,
        regex,
        exit,
        timeout_secs: timeout,
    };
    crate::cli::request_and_handle(session, &req, false, |resp| match resp {
        Response::WaitMatch { line } => {
            println!("{}", line);
            Some(0)
        }
        Response::WaitExited { exit_code } => {
            match exit_code {
                Some(code) => println!("exited with code {}", code),
                None => println!("exited by signal"),
            }
            Some(0)
        }
        Response::WaitTimeout => {
            eprintln!("timeout");
            Some(1)
        }
        _ => None,
    })
    .await
}
