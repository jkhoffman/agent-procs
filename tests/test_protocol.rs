use agent_procs::protocol::*;

#[test]
fn test_run_request_roundtrip() {
    let req = Request::Run { command: "npm run dev".into(), name: Some("webserver".into()), cwd: None };
    let json = serde_json::to_string(&req).unwrap();
    let decoded: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn test_status_response_roundtrip() {
    let resp = Response::Status {
        processes: vec![ProcessInfo {
            name: "webserver".into(), id: "p1".into(), pid: 12345,
            state: ProcessState::Running, exit_code: None,
            uptime_secs: Some(150), command: "npm run dev".into(),
        }],
    };
    let json = serde_json::to_string(&resp).unwrap();
    let decoded: Response = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, decoded);
}

#[test]
fn test_wait_request_with_pattern() {
    let req = Request::Wait {
        target: "webserver".into(), until: Some("Listening on".into()),
        regex: false, exit: false, timeout_secs: Some(30),
    };
    let json = serde_json::to_string(&req).unwrap();
    let decoded: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn test_error_response() {
    let resp = Response::Error { code: 2, message: "process not found: foo".into() };
    let json = serde_json::to_string(&resp).unwrap();
    let decoded: Response = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, decoded);
}
