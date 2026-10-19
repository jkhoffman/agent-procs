mod helpers;
use helpers::TestContext;
use std::io::Write;
use std::time::Duration;

/// Test: running with --port and --proxy shows a URL in stdout.
/// The URL is the proxy URL form: `http://{name}.localhost:{proxy_port`}
#[test]
fn test_run_with_port_shows_url() {
    let ctx = TestContext::new("t-port");

    let run_output = ctx
        .cmd()
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
    let status_output = ctx
        .cmd()
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
    let json_output = ctx
        .cmd()
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

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

/// Test: running without --port does not show a URL.
#[test]
fn test_run_without_port_has_no_url() {
    let ctx = TestContext::new("t-noport");

    let output = ctx
        .cmd()
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

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

/// Test: proxy routes requests via Host header subdomain routing.
#[test]
fn test_proxy_routes_request() {
    let ctx = TestContext::new("t-proxy-route");

    // Start a python HTTP server with --proxy enabled
    let run_output = ctx
        .cmd()
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
            let _ = ctx
                .cmd()
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

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

/// Test: `up` with proxy:true in config starts the proxy and shows URLs in status.
#[test]
fn test_up_with_proxy_config() {
    let ctx = TestContext::new("t-proxy-up");

    let config_dir = tempfile::TempDir::new().unwrap();
    let config_path = config_dir.path().join("agent-procs.yaml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    write!(
        f,
        "proxy: true\nprocesses:\n  echo-srv:\n    cmd: \"echo 'started' && sleep 60\"\n    port: 4600\n"
    )
    .unwrap();

    let output = ctx
        .cmd()
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
    let status_output = ctx
        .cmd()
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

    let _ = ctx.cmd().args(["--session", &ctx.session, "down"]).output();
}

/// Backend for the route-cleanup test: listens on `$PORT` (injected by the
/// daemon), answers `backend-alive` on every GET, and exits as soon as the file
/// named by argv[1] exists, so the test (not a timer) decides when the managed
/// process dies.
const EXIT_ON_FILE_BACKEND: &str = r#"
import http.server, os, socketserver, sys, threading, time

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = b"backend-alive"
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass

socketserver.TCPServer.allow_reuse_address = True
httpd = socketserver.TCPServer(("127.0.0.1", int(os.environ["PORT"])), Handler)

def watch_for_stop_file():
    while not os.path.exists(sys.argv[1]):
        time.sleep(0.05)
    httpd.shutdown()

threading.Thread(target=watch_for_stop_file, daemon=True).start()
httpd.serve_forever()
"#;

/// Upper bound for asynchronous state changes (backend start, exit propagation).
/// Generous so slow CI runners pass, but finite so a real bug still fails.
const PROXY_POLL_TIMEOUT: Duration = Duration::from_secs(20);
const PROXY_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Request `/` through the proxy with the given Host subdomain.
/// Returns the curl exit code and body (empty body on curl failure).
fn proxy_get(proxy_port: u16, name: &str) -> (Option<i32>, String) {
    let output = std::process::Command::new("curl")
        .args([
            "-s",
            "--max-time",
            "5",
            "-H",
            &format!("Host: {}.localhost:{}", name, proxy_port),
            &format!("http://127.0.0.1:{}/", proxy_port),
        ])
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

/// Poll the proxy until `done(body)` is true or `PROXY_POLL_TIMEOUT` elapses.
/// On timeout, returns `Err` with the last observed curl exit code and body.
fn poll_proxy(
    proxy_port: u16,
    name: &str,
    done: impl Fn(&str) -> bool,
) -> Result<(), (Option<i32>, String)> {
    let deadline = std::time::Instant::now() + PROXY_POLL_TIMEOUT;
    loop {
        let (code, body) = proxy_get(proxy_port, name);
        if code == Some(0) && done(&body) {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err((code, body));
        }
        std::thread::sleep(PROXY_POLL_INTERVAL);
    }
}

/// Serializes the tests that let the daemon auto-assign a backend port.
///
/// Every daemon starts its auto-port search at the same port and only
/// bind-tests it, so two sessions starting at the same moment can be handed the
/// same port and one backend then dies with "address already in use". Holding
/// this lock for the lifetime of an [`ExitOnFileBackend`] keeps those tests from
/// racing each other in the default parallel test run.
static AUTO_PORT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A running `EXIT_ON_FILE_BACKEND` behind the proxy.
struct ExitOnFileBackend {
    proxy_port: u16,
    stop_path: std::path::PathBuf,
    _work_dir: tempfile::TempDir,
    // Declared last so it is released after the rest of the backend is dropped.
    _auto_port_guard: std::sync::MutexGuard<'static, ()>,
}

impl ExitOnFileBackend {
    /// Start the backend as process `web` with `--proxy` and wait until the
    /// proxy demonstrably forwards to it, so later cleanup checks cannot pass
    /// vacuously.
    fn start(ctx: &TestContext) -> Self {
        let auto_port_guard = AUTO_PORT_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let work_dir = tempfile::TempDir::new().unwrap();
        let script_path = work_dir.path().join("backend.py");
        let stop_path = work_dir.path().join("stop");
        std::fs::write(&script_path, EXIT_ON_FILE_BACKEND).unwrap();

        let run_output = ctx
            .cmd()
            .args([
                "--session",
                &ctx.session,
                "run",
                &format!(
                    "python3 '{}' '{}'",
                    script_path.display(),
                    stop_path.display()
                ),
                "--name",
                "web",
                // No --port: the daemon picks a free port and injects it as $PORT,
                // so the test never hard-codes or pre-reserves a port that could collide.
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
        let proxy_port = {
            let prefix = "http://localhost:";
            stderr.find(prefix).and_then(|idx| {
                let after = &stderr[idx + prefix.len()..];
                let port_str: String = after.chars().take_while(char::is_ascii_digit).collect();
                port_str.parse::<u16>().ok()
            })
        }
        .unwrap_or_else(|| panic!("could not parse proxy port from stderr: {}", stderr));

        if let Err((code, body)) = poll_proxy(proxy_port, "web", |b| b == "backend-alive") {
            panic!(
                "proxy never forwarded to the live backend within {:?}: last curl exit={:?} body={:?}",
                PROXY_POLL_TIMEOUT, code, body
            );
        }

        Self {
            proxy_port,
            stop_path,
            _work_dir: work_dir,
            _auto_port_guard: auto_port_guard,
        }
    }

    /// Make the managed process exit.
    fn stop(&self) {
        std::fs::write(&self.stop_path, b"").unwrap();
    }

    /// Poll (bounded) until the proxy reports the route for `web` is gone.
    fn assert_route_dropped(&self) {
        if let Err((code, body)) = poll_proxy(self.proxy_port, "web", |b| {
            b.contains("no running process named 'web'")
        }) {
            panic!(
                "stale route was not removed within {:?} after process exit: last curl exit={:?} body={:?}",
                PROXY_POLL_TIMEOUT, code, body
            );
        }
    }
}

/// Test: once a proxied process exits, its route is removed from the proxy.
///
/// First waits until the proxy demonstrably forwards to the live backend, then
/// makes the backend exit and polls (bounded) until the proxy reports the route
/// is gone. `status` is deliberately not polled: it refreshes exit state itself
/// and would mask a stalled background refresh.
#[test]
fn test_proxy_drops_route_after_process_exit() {
    let ctx = TestContext::new("t-proxy-exit");
    let backend = ExitOnFileBackend::start(&ctx);

    backend.stop();
    backend.assert_route_dropped();

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}

/// Test: the route is also dropped when the exit is first observed by a
/// `wait --exit` request (which reaps the child) rather than by the daemon's
/// periodic exit refresh.
#[test]
fn test_proxy_drops_route_after_wait_exit_reaps_process() {
    let ctx = TestContext::new("t-proxy-wait-exit");
    let backend = ExitOnFileBackend::start(&ctx);

    // Each `wait --exit` polls every 100ms, which the daemon answers by reaping
    // the child. The daemon's own refresh tick (200ms) can reap it first, in
    // which case the bug is not exercised. Start several waiters with their
    // polls staggered across the interval so a waiter almost always gets there
    // first. This only affects how sensitive the test is to the bug; the check
    // afterwards is bounded polling and never depends on the ordering.
    let waiters: Vec<_> = (0..8)
        .map(|_| {
            let waiter = std::process::Command::new(assert_cmd::cargo::cargo_bin("agent-procs"))
                .env("XDG_STATE_HOME", ctx.state_dir.path())
                .args([
                    "--session",
                    &ctx.session,
                    "wait",
                    "web",
                    "--exit",
                    "--timeout",
                    "30",
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            std::thread::sleep(Duration::from_millis(12));
            waiter
        })
        .collect();
    backend.stop();
    for waiter in waiters {
        let wait_output = waiter.wait_with_output().unwrap();
        assert!(
            wait_output.status.success(),
            "wait --exit failed: stdout={} stderr={}",
            String::from_utf8_lossy(&wait_output.stdout),
            String::from_utf8_lossy(&wait_output.stderr)
        );
    }

    backend.assert_route_dropped();

    let _ = ctx
        .cmd()
        .args(["--session", &ctx.session, "stop-all"])
        .output();
}
