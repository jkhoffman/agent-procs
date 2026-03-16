use agent_procs::protocol::*;

#[test]
fn test_run_request_roundtrip() {
    let req = Request::Run {
        command: "npm run dev".into(),
        name: Some("webserver".into()),
        cwd: None,
        env: Some(std::collections::HashMap::from([(
            "NODE_ENV".to_string(),
            "production".to_string(),
        )])),
        port: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    let decoded: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn test_status_response_roundtrip() {
    let resp = Response::Status {
        processes: vec![ProcessInfo {
            name: "webserver".into(),
            id: "p1".into(),
            pid: 12345,
            state: ProcessState::Running,
            exit_code: None,
            uptime_secs: Some(150),
            command: "npm run dev".into(),
            port: None,
            url: None,
        }],
    };
    let json = serde_json::to_string(&resp).unwrap();
    let decoded: Response = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, decoded);
}

#[test]
fn test_wait_request_with_pattern() {
    let req = Request::Wait {
        target: "webserver".into(),
        until: Some("Listening on".into()),
        regex: false,
        exit: false,
        timeout_secs: Some(30),
    };
    let json = serde_json::to_string(&req).unwrap();
    let decoded: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn test_error_response() {
    let resp = Response::Error {
        code: 2,
        message: "process not found: foo".into(),
    };
    let json = serde_json::to_string(&resp).unwrap();
    let decoded: Response = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, decoded);
}

#[test]
fn test_run_request_with_port() {
    let req = Request::Run {
        command: "npm run dev".into(),
        name: Some("webserver".into()),
        cwd: None,
        env: None,
        port: Some(3000),
    };
    let json = serde_json::to_string(&req).unwrap();
    let decoded: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, decoded);
    // Ensure backward compat: absent port field defaults to None
    let no_port = r#"{"type":"Run","command":"npm run dev","name":"webserver"}"#;
    let decoded2: Request = serde_json::from_str(no_port).unwrap();
    if let Request::Run { port, .. } = decoded2 {
        assert_eq!(port, None);
    } else {
        panic!("expected Run variant");
    }
}

#[test]
fn test_enable_proxy_with_port() {
    let req = Request::EnableProxy {
        proxy_port: Some(8080),
    };
    let json = serde_json::to_string(&req).unwrap();
    let decoded: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn test_enable_proxy_without_port() {
    let req = Request::EnableProxy { proxy_port: None };
    let json = serde_json::to_string(&req).unwrap();
    let decoded: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, decoded);
    // Ensure absent field deserializes correctly from minimal JSON
    let minimal = r#"{"type":"EnableProxy"}"#;
    let decoded2: Request = serde_json::from_str(minimal).unwrap();
    assert_eq!(decoded2, req);
}

#[test]
fn test_run_ok_with_port_and_url() {
    let resp = Response::RunOk {
        name: "web".into(),
        id: "p1".into(),
        pid: 42,
        port: Some(3000),
        url: Some("http://localhost:3000".into()),
    };
    let json = serde_json::to_string(&resp).unwrap();
    let decoded: Response = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, decoded);
}

#[test]
fn test_run_ok_without_port() {
    let resp = Response::RunOk {
        name: "web".into(),
        id: "p1".into(),
        pid: 42,
        port: None,
        url: None,
    };
    let json = serde_json::to_string(&resp).unwrap();
    let decoded: Response = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, decoded);
    // Ensure absent fields deserialize correctly from minimal JSON
    let minimal = r#"{"type":"RunOk","name":"web","id":"p1","pid":42}"#;
    let decoded2: Response = serde_json::from_str(minimal).unwrap();
    assert_eq!(decoded2, resp);
}
