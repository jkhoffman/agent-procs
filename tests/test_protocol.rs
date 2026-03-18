use agent_procs::protocol::*;
use std::collections::HashMap;

#[test]
fn test_run_request_roundtrip() {
    let req = Request::Run {
        command: "npm run dev".into(),
        name: Some("webserver".into()),
        cwd: None,
        env: Some(HashMap::from([(
            "NODE_ENV".to_string(),
            "production".to_string(),
        )])),
        port: None,
        restart: None,
        watch: None,
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
        code: ErrorCode::NotFound,
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
        restart: None,
        watch: None,
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

#[test]
fn test_restart_policy_serde_roundtrip() {
    let policy = RestartPolicy {
        mode: RestartMode::OnFailure,
        max_restarts: Some(5),
        restart_delay_ms: 2000,
    };
    let json = serde_json::to_string(&policy).unwrap();
    let parsed: RestartPolicy = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, policy);
}

#[test]
fn test_watch_config_serde_roundtrip() {
    let config = WatchConfig {
        paths: vec!["src/**".into(), "config/*".into()],
        ignore: Some(vec!["*.log".into()]),
    };
    let json = serde_json::to_string(&config).unwrap();
    let parsed: WatchConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, config);
}

#[test]
fn test_run_request_with_restart_and_watch() {
    let req = Request::Run {
        command: "npm start".into(),
        name: Some("server".into()),
        cwd: None,
        env: None,
        port: None,
        restart: Some(RestartPolicy {
            mode: RestartMode::OnFailure,
            max_restarts: Some(5),
            restart_delay_ms: 1000,
        }),
        watch: Some(WatchConfig {
            paths: vec!["src/**".into()],
            ignore: None,
        }),
    };
    let json = serde_json::to_string(&req).unwrap();
    let parsed: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, req);
}

#[test]
fn test_run_request_without_restart_watch_backward_compat() {
    // Old-style Run request without restart/watch fields should parse fine
    let json = r#"{"type":"Run","command":"ls","name":null,"cwd":null,"env":null,"port":null}"#;
    let parsed: Request = serde_json::from_str(json).unwrap();
    if let Request::Run { restart, watch, .. } = parsed {
        assert!(restart.is_none());
        assert!(watch.is_none());
    } else {
        panic!("expected Run");
    }
}
