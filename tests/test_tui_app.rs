use agent_procs::tui::app::*;
use agent_procs::protocol::{ProcessInfo, ProcessState, Stream};

#[test]
fn test_output_buffer_ring_behavior() {
    let mut buf = OutputBuffer::new(5); // max 5 lines
    for i in 0..8 {
        buf.push(LineSource::Stdout, format!("line {}", i));
    }
    let lines = buf.stdout_lines();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0], "line 3");
    assert_eq!(lines[4], "line 7");
}

#[test]
fn test_output_buffer_stderr() {
    let mut buf = OutputBuffer::new(100);
    buf.push(LineSource::Stdout, "out1".into());
    buf.push(LineSource::Stderr, "err1".into());
    buf.push(LineSource::Stdout, "out2".into());

    assert_eq!(buf.stdout_lines().len(), 2);
    assert_eq!(buf.stderr_lines().len(), 1);
    assert_eq!(buf.all_lines().len(), 3);
    assert_eq!(buf.all_lines()[1].0, LineSource::Stderr);
}

#[test]
fn test_app_select_next_wraps() {
    let mut app = App::new();
    app.update_processes(vec![
        make_info("alpha", ProcessState::Running),
        make_info("beta", ProcessState::Running),
    ]);
    assert_eq!(app.selected, 0);
    app.select_next();
    assert_eq!(app.selected, 1);
    app.select_next();
    assert_eq!(app.selected, 0); // wraps
}

#[test]
fn test_app_select_prev_wraps() {
    let mut app = App::new();
    app.update_processes(vec![
        make_info("alpha", ProcessState::Running),
        make_info("beta", ProcessState::Running),
    ]);
    assert_eq!(app.selected, 0);
    app.select_prev();
    assert_eq!(app.selected, 1); // wraps to end
}

#[test]
fn test_stream_mode_cycles() {
    let mut app = App::new();
    assert!(matches!(app.stream_mode, StreamMode::Stdout));
    app.cycle_stream_mode();
    assert!(matches!(app.stream_mode, StreamMode::Stderr));
    app.cycle_stream_mode();
    assert!(matches!(app.stream_mode, StreamMode::Both));
    app.cycle_stream_mode();
    assert!(matches!(app.stream_mode, StreamMode::Stdout));
}

#[test]
fn test_pause_toggle() {
    let mut app = App::new();
    assert!(!app.paused);
    app.toggle_pause();
    assert!(app.paused);
    app.toggle_pause();
    assert!(!app.paused);
}

#[test]
fn test_push_output_creates_buffer() {
    let mut app = App::new();
    app.push_output("server", Stream::Stdout, "hello");
    assert!(app.buffers.contains_key("server"));
    let buf = &app.buffers["server"];
    assert_eq!(buf.stdout_lines().len(), 1);
}

fn make_info(name: &str, state: ProcessState) -> ProcessInfo {
    ProcessInfo {
        name: name.into(), id: "p1".into(), pid: 1234,
        state, exit_code: None, uptime_secs: Some(10),
        command: "test".into(),
    }
}
