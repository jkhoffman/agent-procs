mod helpers;
use helpers::TestContext;
use nix::sys::signal::{kill, killpg};
use nix::unistd::Pid;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

const CMD_TIMEOUT: Duration = Duration::from_secs(20);

/// A process group whose shell and grandchild both ignore SIGTERM. The
/// grandchild writes its pid to `pid_file`.
fn stubborn_cmd(pid_file: &Path) -> String {
    format!(
        "trap '' TERM; sh -c 'trap \"\" TERM; echo $$ > {}; while :; do sleep 0.1; done' & while :; do sleep 0.1; done",
        pid_file.display()
    )
}

fn alive(pid: i32) -> bool {
    kill(Pid::from_raw(pid), None).is_ok()
}

fn group_alive(pgid: i32) -> bool {
    killpg(Pid::from_raw(pgid), None).is_ok()
}

fn wait_for_file(path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(s) = std::fs::read_to_string(path)
            && !s.trim().is_empty()
        {
            return s;
        }
        assert!(
            Instant::now() < deadline,
            "{} never written",
            path.display()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn read_pid(path: &Path) -> i32 {
    wait_for_file(path)
        .lines()
        .last()
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

fn run_ok(ctx: &TestContext, cmd: &str, name: &str, extra: &[&str]) -> i32 {
    let mut args = vec!["--session", &ctx.session, "run", cmd, "--name", name];
    args.extend_from_slice(extra);
    let out = ctx.cmd().args(&args).timeout(CMD_TIMEOUT).output().unwrap();
    assert!(
        out.status.success(),
        "run {name} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let pid = stdout.split("pid: ").nth(1).unwrap();
    pid.trim_end_matches(|c: char| !c.is_ascii_digit())
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

fn status(ctx: &TestContext) -> Vec<serde_json::Value> {
    let out = ctx
        .cmd()
        .args(["--session", &ctx.session, "status", "--json"])
        .timeout(CMD_TIMEOUT)
        .output()
        .unwrap();
    assert!(out.status.success(), "status failed");
    serde_json::from_slice(&out.stdout).unwrap()
}

fn state_of(ctx: &TestContext, name: &str) -> Option<String> {
    status(ctx)
        .iter()
        .find(|p| p["name"] == name)
        .map(|p| p["state"].as_str().unwrap().to_string())
}

fn log_dir(ctx: &TestContext) -> std::path::PathBuf {
    let dir = ctx
        .state_dir
        .path()
        .join("agent-procs/sessions")
        .join(&ctx.session)
        .join("logs");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn timed(ctx: &TestContext, args: &[&str]) -> (std::process::Output, Duration) {
    let mut full = vec!["--session", ctx.session.as_str()];
    full.extend_from_slice(args);
    let start = Instant::now();
    let out = ctx.cmd().args(&full).timeout(CMD_TIMEOUT).output().unwrap();
    (out, start.elapsed())
}

fn assert_no_marker_process(marker: &str) {
    let out = std::process::Command::new("pgrep")
        .args(["-f", marker])
        .output()
        .unwrap();
    assert!(
        out.stdout.is_empty(),
        "a process with marker {marker} is still alive: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

fn assert_run_fails_without_child(ctx: &TestContext, blocker: &str) {
    let name = "victim";
    std::fs::create_dir(log_dir(ctx).join(format!("{name}.{blocker}"))).unwrap();
    let marker = format!("ap-marker-{}-{}", ctx.session, blocker.replace('.', "-"));
    let cmd = format!("while :; do sleep 0.1; done # {marker}");
    let (out, _) = timed(ctx, &["run", &cmd, "--name", name]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "run must fail when its log cannot be opened"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("log"),
        "error should name the log: {stderr}"
    );
    thread::sleep(Duration::from_millis(300));
    assert_no_marker_process(&marker);
    assert_eq!(
        state_of(ctx, name),
        None,
        "failed run must not leave a record"
    );
}

#[test]
fn run_fails_when_stdout_log_cannot_open() {
    let ctx = TestContext::new("t-logfail-stdout");
    assert_run_fails_without_child(&ctx, "stdout");
}

#[test]
fn run_fails_when_stderr_index_cannot_open() {
    let ctx = TestContext::new("t-logfail-idx");
    assert_run_fails_without_child(&ctx, "stderr.idx");
}

#[test]
fn restart_fails_when_log_cannot_open() {
    let ctx = TestContext::new("t-logfail-restart");
    let marker = format!("ap-marker-{}", ctx.session);
    let cmd = format!("while :; do sleep 0.1; done # {marker}");
    run_ok(&ctx, &cmd, "victim", &[]);
    let log = log_dir(&ctx).join("victim.stdout");
    std::fs::remove_file(&log).unwrap();
    std::fs::create_dir(&log).unwrap();

    let (out, _) = timed(&ctx, &["restart", "victim"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "restart must fail when its log cannot be opened"
    );
    thread::sleep(Duration::from_millis(300));
    assert_no_marker_process(&marker);
    assert_eq!(state_of(&ctx, "victim").as_deref(), Some("failed"));
}

#[test]
fn slow_stop_does_not_block_other_clients() {
    let ctx = TestContext::new("t-slowstop");
    let gc_file = ctx.state_dir.path().join("gc.pid");
    let leader = run_ok(&ctx, &stubborn_cmd(&gc_file), "stubborn", &[]);
    let grandchild = read_pid(&gc_file);

    let session = ctx.session.clone();
    let state_home = ctx.state_dir.path().to_path_buf();
    let stopper = thread::spawn(move || {
        let start = Instant::now();
        let out = assert_cmd::Command::cargo_bin("agent-procs")
            .unwrap()
            .env("XDG_STATE_HOME", state_home)
            .args(["--session", &session, "stop", "stubborn"])
            .timeout(CMD_TIMEOUT)
            .output()
            .unwrap();
        (out, start.elapsed())
    });

    thread::sleep(Duration::from_millis(300));
    let (out, took) = timed(&ctx, &["status", "--json"]);
    assert!(out.status.success());
    assert!(
        took < Duration::from_secs(1),
        "status blocked for {took:?} during a stop"
    );
    let procs: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    let p = procs.iter().find(|p| p["name"] == "stubborn").unwrap();
    assert_eq!(p["state"], "running", "a stopping process is still running");

    let (stop_out, stop_took) = stopper.join().unwrap();
    assert!(stop_out.status.success());
    assert!(
        stop_took >= Duration::from_millis(1500) && stop_took < Duration::from_secs(6),
        "stop should escalate after the 2s grace, took {stop_took:?}"
    );
    assert!(!alive(grandchild), "TERM-ignoring grandchild survived stop");
    assert!(!group_alive(leader), "process group survived stop");
    assert_eq!(state_of(&ctx, "stubborn").as_deref(), Some("exited"));
}

#[test]
fn stop_after_leader_exit_kills_remaining_group() {
    let ctx = TestContext::new("t-leaderexit");
    let gc_file = ctx.state_dir.path().join("gc.pid");
    let cmd = format!(
        "sh -c 'trap \"\" TERM; echo $$ > {}; while :; do sleep 0.1; done' & echo leader-exiting",
        gc_file.display()
    );
    let leader = run_ok(&ctx, &cmd, "leaky", &[]);
    let grandchild = read_pid(&gc_file);
    thread::sleep(Duration::from_millis(600));
    assert_eq!(state_of(&ctx, "leaky").as_deref(), Some("exited"));
    assert!(alive(grandchild));

    let (out, _) = timed(&ctx, &["stop", "leaky"]);
    assert!(out.status.success());
    assert!(
        !alive(grandchild),
        "grandchild survived stop of an exited leader"
    );
    assert!(
        !group_alive(leader),
        "group (or unreaped leader) survived stop"
    );
}

#[test]
fn autorestart_with_lingering_grandchild_does_not_wedge_daemon() {
    let ctx = TestContext::new("t-arwedge");
    let gc_file = ctx.state_dir.path().join("gc.pids");
    let cmd = format!(
        "sh -c 'echo $$ >> {}; exec sleep 30' & echo up; exit 1",
        gc_file.display()
    );
    run_ok(
        &ctx,
        &cmd,
        "crashy",
        &[
            "--autorestart",
            "always",
            "--max-restarts",
            "2",
            "--restart-delay",
            "200",
        ],
    );

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let (out, took) = timed(&ctx, &["status", "--json"]);
        assert!(out.status.success());
        assert!(took < Duration::from_secs(2), "status blocked for {took:?}");
        let procs: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
        let p = procs.iter().find(|p| p["name"] == "crashy").unwrap();
        if p["state"] == "failed" {
            break;
        }
        assert!(Instant::now() < deadline, "restarts never exhausted: {p}");
        thread::sleep(Duration::from_millis(200));
    }

    let log = std::fs::read_to_string(log_dir(&ctx).join("crashy.stdout")).unwrap();
    assert!(
        log.contains("[agent-procs] Restarted"),
        "no restart marker: {log}"
    );

    let pids: Vec<i32> = wait_for_file(&gc_file)
        .lines()
        .map(|l| l.trim().parse().unwrap())
        .collect();
    assert_eq!(pids.len(), 3, "expected 3 incarnations, got {pids:?}");
    for pid in &pids[..2] {
        assert!(
            !alive(*pid),
            "grandchild {pid} of a restarted incarnation survived"
        );
    }

    let (out, _) = timed(&ctx, &["stop", "crashy"]);
    assert!(out.status.success());
    assert!(!alive(pids[2]), "last grandchild survived stop");
}

#[test]
fn concurrent_stops_both_succeed() {
    let ctx = TestContext::new("t-dblstop");
    let gc_file = ctx.state_dir.path().join("gc.pid");
    run_ok(&ctx, &stubborn_cmd(&gc_file), "stubborn", &[]);
    read_pid(&gc_file);

    let handles: Vec<_> = (0..2)
        .map(|_| {
            let session = ctx.session.clone();
            let state_home = ctx.state_dir.path().to_path_buf();
            thread::spawn(move || {
                assert_cmd::Command::cargo_bin("agent-procs")
                    .unwrap()
                    .env("XDG_STATE_HOME", state_home)
                    .args(["--session", &session, "stop", "stubborn"])
                    .timeout(CMD_TIMEOUT)
                    .output()
                    .unwrap()
            })
        })
        .collect();
    for h in handles {
        let out = h.join().unwrap();
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "stopped stubborn"
        );
    }
    assert_eq!(state_of(&ctx, "stubborn").as_deref(), Some("exited"));
}

#[test]
fn restart_during_stop_wins_and_stop_during_restart_wins() {
    let ctx = TestContext::new("t-overlap");
    let gc_file = ctx.state_dir.path().join("gc.pid");
    run_ok(&ctx, &stubborn_cmd(&gc_file), "stubborn", &[]);
    read_pid(&gc_file);

    let spawn_cmd = |args: Vec<&'static str>| {
        let session = ctx.session.clone();
        let state_home = ctx.state_dir.path().to_path_buf();
        thread::spawn(move || {
            let mut full = vec!["--session".to_string(), session];
            full.extend(args.iter().map(|s| (*s).to_string()));
            assert_cmd::Command::cargo_bin("agent-procs")
                .unwrap()
                .env("XDG_STATE_HOME", state_home)
                .args(&full)
                .timeout(CMD_TIMEOUT)
                .output()
                .unwrap()
        })
    };

    let stop = spawn_cmd(vec!["stop", "stubborn"]);
    thread::sleep(Duration::from_millis(300));
    let (restart, _) = timed(&ctx, &["restart", "stubborn"]);
    assert!(stop.join().unwrap().status.success());
    assert!(
        restart.status.success(),
        "a restart after a stop should respawn"
    );
    assert_eq!(state_of(&ctx, "stubborn").as_deref(), Some("running"));
    read_pid(&gc_file);

    let restart = spawn_cmd(vec!["restart", "stubborn"]);
    thread::sleep(Duration::from_millis(300));
    let (stop, _) = timed(&ctx, &["stop", "stubborn"]);
    assert!(stop.status.success());
    let restart = restart.join().unwrap();
    assert_eq!(
        restart.status.code(),
        Some(1),
        "a stop after a restart supersedes it"
    );
    assert!(String::from_utf8_lossy(&restart.stderr).contains("superseded"));
    assert_eq!(state_of(&ctx, "stubborn").as_deref(), Some("exited"));
}

#[test]
fn stop_all_stops_groups_concurrently() {
    let ctx = TestContext::new("t-stopall");
    let mut groups = Vec::new();
    for i in 0..3 {
        let gc_file = ctx.state_dir.path().join(format!("gc{i}.pid"));
        let leader = run_ok(&ctx, &stubborn_cmd(&gc_file), &format!("s{i}"), &[]);
        groups.push((leader, read_pid(&gc_file)));
    }

    let session = ctx.session.clone();
    let state_home = ctx.state_dir.path().to_path_buf();
    let stopper = thread::spawn(move || {
        let start = Instant::now();
        let out = assert_cmd::Command::cargo_bin("agent-procs")
            .unwrap()
            .env("XDG_STATE_HOME", state_home)
            .args(["--session", &session, "stop-all"])
            .timeout(Duration::from_secs(40))
            .output()
            .unwrap();
        (out, start.elapsed())
    });

    thread::sleep(Duration::from_millis(300));
    let (run, took) = timed(&ctx, &["run", "sleep 30", "--name", "late"]);
    assert!(
        took < Duration::from_secs(1),
        "run blocked for {took:?} during stop-all"
    );
    assert_eq!(run.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&run.stderr).contains("stop-all in progress"));

    let (out, took) = stopper.join().unwrap();
    assert!(out.status.success());
    assert!(took < Duration::from_secs(6), "stop-all took {took:?}");
    for (leader, grandchild) in groups {
        assert!(
            !alive(grandchild),
            "grandchild {grandchild} survived stop-all"
        );
        assert!(!group_alive(leader), "group {leader} survived stop-all");
    }
    let left = status(&ctx);
    assert!(left.is_empty(), "records left after stop-all: {left:?}");
}
