mod helpers;
use assert_cmd::Command;
use helpers::TestContext;
use std::io::Write;
use std::time::Duration;

#[test]
fn test_up_starts_all_processes() {
    let ctx = TestContext::new("test-up-all");
    ctx.set_env();

    let config_dir = tempfile::TempDir::new().unwrap();
    let config_path = config_dir.path().join("agent-procs.yaml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    write!(f, "processes:\n  alpha:\n    cmd: \"echo alpha-ready && sleep 60\"\n    ready: \"alpha-ready\"\n  beta:\n    cmd: \"echo beta-ready && sleep 60\"\n    ready: \"beta-ready\"\n").unwrap();

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
        "up failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "status"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("alpha"));
    assert!(stdout.contains("beta"));

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "down"])
        .output();
}

#[test]
fn test_up_respects_depends_on() {
    let ctx = TestContext::new("test-up-deps");
    ctx.set_env();

    let config_dir = tempfile::TempDir::new().unwrap();
    let config_path = config_dir.path().join("agent-procs.yaml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    write!(f, "processes:\n  db:\n    cmd: \"echo db-ready && sleep 60\"\n    ready: \"db-ready\"\n  api:\n    cmd: \"echo api-ready && sleep 60\"\n    ready: \"api-ready\"\n    depends_on: [db]\n").unwrap();

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
    assert!(output.status.success());

    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "status"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("db"));
    assert!(stdout.contains("api"));

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "down"])
        .output();
}

#[test]
fn test_up_with_env_and_cwd() {
    let ctx = TestContext::new("t-up-env");
    ctx.set_env();

    let config_dir = tempfile::TempDir::new().unwrap();
    let sub = config_dir.path().join("subdir");
    std::fs::create_dir_all(&sub).unwrap();

    let config_path = config_dir.path().join("agent-procs.yaml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    write!(
        f,
        r#"processes:
  worker:
    cmd: "echo MY_VAR=$MY_VAR CWD=$(pwd) ready && sleep 60"
    cwd: ./subdir
    env:
      MY_VAR: hello123
    ready: "ready"
"#
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
        "up failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    std::thread::sleep(Duration::from_millis(300));

    let output = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "logs", "worker", "--tail", "5"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("MY_VAR=hello123"),
        "env var not set, got: {}",
        stdout
    );
    assert!(
        stdout.contains("subdir"),
        "cwd not applied, got: {}",
        stdout
    );

    let _ = Command::cargo_bin("agent-procs")
        .unwrap()
        .args(["--session", &ctx.session, "down"])
        .output();
}
