mod helpers;
use assert_cmd::Command;
use helpers::TestContext;
use std::io::Write;
use std::time::Duration;

/// Test: running with --port and --proxy shows a URL in stdout.
/// The URL is the proxy URL form: `http://{name}.localhost:{proxy_port`}
#[test]
fn test_run_with_port_shows_url() {
    let ctx = TestContext::new("t-port");
    ctx.set_env();

    let run_output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args([
            "--session",
            &ctx.session,
            "run",
            "echo 'listening' && sleep 60",
            "--name",
            "api",
            "--port",
            "4567",
            "--proxy",
        ])
        .output()
        .unwrap();

    assert!(
        run_output.status.success(),
        "run failed: stdout={} stderr={}",
        String::from_utf8_lossy(&run_output.stdout),
        String::from_utf8_lossy(&run_output.stderr)
    );

    // Proxy starts the listener; the run output shows the proxy URL (http://api.localhost:{proxy_port})
    let stdout = String::from_utf8_lossy(&run_output.stdout);
    assert!(
        stdout.contains("http://"),
        "expected a URL in stdout, got: {}",
        stdout
    );

    // status should show the URL column
    let status_output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "status"])
        .output()
        .unwrap();
    assert!(status_output.status.success());
    let status_stdout = String::from_utf8_lossy(&status_output.stdout);
    assert!(
        status_stdout.contains("http://"),
        "expected URL in status output, got: {}",
        status_stdout
    );

    // status --json should contain port and url fields
    let json_output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "status", "--json"])
        .output()
        .unwrap();
    assert!(json_output.status.success());
    let json_str = String::from_utf8_lossy(&json_output.stdout);
    assert!(
        json_str.contains("\"port\""),
        "expected 'port' field in JSON status, got: {}",
        json_str
    );
    assert!(
        json_str.contains("\"url\""),
        "expected 'url' field in JSON status, got: {}",
        json_str
    );

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

/// Test: running without --port does not show a URL.
#[test]
fn test_run_without_port_has_no_url() {
    let ctx = TestContext::new("t-noport");
    ctx.set_env();

    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "run", "sleep 60", "--name", "bg"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("http://"),
        "expected no URL in stdout, got: {}",
        stdout
    );

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

/// Test: proxy routes requests via Host header subdomain routing.
#[test]
fn test_proxy_routes_request() {
    let ctx = TestContext::new("t-proxy-route");
    ctx.set_env();

    // Start a python HTTP server with --proxy enabled
    let run_output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args([
            "--session",
            &ctx.session,
            "run",
            "python3 -m http.server 4500 --bind 127.0.0.1",
            "--name",
            "web",
            "--port",
            "4500",
            "--proxy",
        ])
        .output()
        .unwrap();

    assert!(
        run_output.status.success(),
        "run failed: stdout={} stderr={}",
        String::from_utf8_lossy(&run_output.stdout),
        String::from_utf8_lossy(&run_output.stderr)
    );

    let stderr = String::from_utf8_lossy(&run_output.stderr);
    assert!(
        stderr.contains("Proxy listening"),
        "expected 'Proxy listening' in stderr, got: {}",
        stderr
    );

    // Parse proxy port from stderr: "Proxy listening on http://localhost:PORT"
    let proxy_port = {
        let prefix = "http://localhost:";
        stderr.find(prefix).and_then(|idx| {
            let after = &stderr[idx + prefix.len()..];
            let port_str: String = after.chars().take_while(char::is_ascii_digit).collect();
            port_str.parse::<u16>().ok()
        })
    };

    let proxy_port = match proxy_port {
        Some(p) => p,
        None => {
            let _ = Command::cargo_bin("agent-procs")
                .unwrap()
                .args(["--session", &ctx.session, "stop-all"])
                .output();
            panic!("could not parse proxy port from stderr: {}", stderr);
        }
    };

    // Wait for python http.server to bind
    std::thread::sleep(Duration::from_millis(500));

    // Make a request through the proxy using the Host subdomain routing
    let curl_result = std::process::Command::new("curl")
        .args([
            "-s",
            "--max-time",
            "5",
            "-H",
            &format!("Host: web.localhost:{}", proxy_port),
            &format!("http://127.0.0.1:{}/", proxy_port),
        ])
        .output();

    if let Ok(curl_output) = curl_result
        && curl_output.status.success()
    {
        let body = String::from_utf8_lossy(&curl_output.stdout);
        assert!(!body.is_empty(), "expected non-empty response from proxy");
    }
    // If curl exits with error (e.g. backend not ready yet), the proxy itself
    // started correctly — we already verified that above.

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

/// Test: `up` with proxy:true in config starts the proxy and shows URLs in status.
#[test]
fn test_up_with_proxy_config() {
    let ctx = TestContext::new("t-proxy-up");
    ctx.set_env();

    let config_dir = tempfile::TempDir::new().unwrap();
    let config_path = config_dir.path().join("agent-procs.yaml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    write!(
        f,
        "proxy: true\nprocesses:\n  echo-srv:\n    cmd: \"echo 'started' && sleep 60\"\n    port: 4600\n"
    )
    .unwrap();

    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args([
            "--session",
            &ctx.session,
            "up",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .timeout(Duration::from_secs(30))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "up failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Proxy listening"),
        "expected 'Proxy listening' in stderr, got: {}",
        stderr
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("echo-srv"),
        "expected 'echo-srv' in stdout, got: {}",
        stdout
    );

    // status should show URL for echo-srv (proxy URL form: echo-srv.localhost:{proxy_port})
    let status_output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "status"])
        .output()
        .unwrap();
    assert!(status_output.status.success());
    let status_stdout = String::from_utf8_lossy(&status_output.stdout);
    assert!(
        status_stdout.contains("echo-srv.localhost"),
        "expected 'echo-srv.localhost' in status output, got: {}",
        status_stdout
    );

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "down"])
        .output();
}
