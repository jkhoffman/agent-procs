use proptest::prelude::*;
use std::collections::HashMap;

use agent_procs::daemon::process_manager::is_valid_dns_label;
use agent_procs::protocol::{ErrorCode, ProcessInfo, ProcessState, Request, Response, Stream};

// --- Strategies for generating arbitrary Request variants ---

fn arb_optional_string() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), any::<String>().prop_map(Some),]
}

fn arb_optional_u16() -> impl Strategy<Value = Option<u16>> {
    prop_oneof![Just(None), any::<u16>().prop_map(Some),]
}

fn arb_optional_u64() -> impl Strategy<Value = Option<u64>> {
    prop_oneof![Just(None), any::<u64>().prop_map(Some),]
}

fn arb_optional_usize() -> impl Strategy<Value = Option<usize>> {
    prop_oneof![Just(None), any::<usize>().prop_map(Some),]
}

fn arb_optional_env() -> impl Strategy<Value = Option<HashMap<String, String>>> {
    prop_oneof![
        Just(None),
        prop::collection::hash_map(any::<String>(), any::<String>(), 0..5).prop_map(Some),
    ]
}

fn arb_request() -> impl Strategy<Value = Request> {
    prop_oneof![
        (
            any::<String>(),
            arb_optional_string(),
            arb_optional_string(),
            arb_optional_env(),
            arb_optional_u16(),
        )
            .prop_map(|(command, name, cwd, env, port)| Request::Run {
                command,
                name,
                cwd,
                env,
                port,
                restart: None,
                watch: None,
            }),
        any::<String>().prop_map(|target| Request::Stop { target }),
        Just(Request::StopAll),
        any::<String>().prop_map(|target| Request::Restart { target }),
        Just(Request::Status),
        (
            arb_optional_string(),
            any::<usize>(),
            any::<bool>(),
            any::<bool>(),
            any::<bool>(),
            arb_optional_u64(),
            arb_optional_usize(),
            arb_optional_string(),
            any::<bool>(),
        )
            .prop_map(
                |(target, tail, follow, stderr, all, timeout_secs, lines, grep, regex)| {
                    Request::Logs {
                        target,
                        tail,
                        follow,
                        stderr,
                        all,
                        timeout_secs,
                        lines,
                        grep,
                        regex,
                    }
                }
            ),
        (
            any::<String>(),
            arb_optional_string(),
            any::<bool>(),
            any::<bool>(),
            arb_optional_u64(),
        )
            .prop_map(|(target, until, regex, exit, timeout_secs)| Request::Wait {
                target,
                until,
                regex,
                exit,
                timeout_secs,
            }),
        Just(Request::Shutdown),
        arb_optional_u16().prop_map(|proxy_port| Request::EnableProxy { proxy_port }),
        any::<u32>().prop_map(|version| Request::Hello { version }),
        Just(Request::Unknown),
    ]
}

// --- Strategies for generating arbitrary Response variants ---

fn arb_optional_i32() -> impl Strategy<Value = Option<i32>> {
    prop_oneof![Just(None), any::<i32>().prop_map(Some),]
}

fn arb_error_code() -> impl Strategy<Value = ErrorCode> {
    prop_oneof![Just(ErrorCode::General), Just(ErrorCode::NotFound),]
}

fn arb_stream() -> impl Strategy<Value = Stream> {
    prop_oneof![Just(Stream::Stdout), Just(Stream::Stderr),]
}

fn arb_process_state() -> impl Strategy<Value = ProcessState> {
    prop_oneof![Just(ProcessState::Running), Just(ProcessState::Exited),]
}

fn arb_process_info() -> impl Strategy<Value = ProcessInfo> {
    (
        any::<String>(),
        any::<String>(),
        any::<u32>(),
        arb_process_state(),
        arb_optional_i32(),
        arb_optional_u64(),
        any::<String>(),
        arb_optional_u16(),
        arb_optional_string(),
    )
        .prop_map(
            |(name, id, pid, state, exit_code, uptime_secs, command, port, url)| ProcessInfo {
                name,
                id,
                pid,
                state,
                exit_code,
                uptime_secs,
                command,
                port,
                url,
                restart_count: None,
                max_restarts: None,
                restart_policy: None,
                watched: None,
            },
        )
}

fn arb_response() -> impl Strategy<Value = Response> {
    prop_oneof![
        any::<String>().prop_map(|message| Response::Ok { message }),
        (
            any::<String>(),
            any::<String>(),
            any::<u32>(),
            arb_optional_u16(),
            arb_optional_string(),
        )
            .prop_map(|(name, id, pid, port, url)| Response::RunOk {
                name,
                id,
                pid,
                port,
                url,
            }),
        prop::collection::vec(arb_process_info(), 0..5)
            .prop_map(|processes| Response::Status { processes }),
        (any::<String>(), arb_stream(), any::<String>()).prop_map(|(process, stream, line)| {
            Response::LogLine {
                process,
                stream,
                line,
            }
        }),
        Just(Response::LogEnd),
        any::<String>().prop_map(|line| Response::WaitMatch { line }),
        arb_optional_i32().prop_map(|exit_code| Response::WaitExited { exit_code }),
        Just(Response::WaitTimeout),
        (arb_error_code(), any::<String>())
            .prop_map(|(code, message)| Response::Error { code, message }),
        any::<u32>().prop_map(|version| Response::Hello { version }),
        Just(Response::Unknown),
    ]
}

// --- Property tests ---

proptest! {
    #[test]
    fn request_roundtrip(req in arb_request()) {
        let json = serde_json::to_string(&req).unwrap();
        let parsed: Request = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(parsed, req);
    }

    #[test]
    fn response_roundtrip(resp in arb_response()) {
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: Response = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(parsed, resp);
    }

    #[test]
    fn dns_label_agrees_with_regex(s in "[a-z0-9\\-]{0,80}") {
        let regex_valid = {
            let re = regex::Regex::new(r"^[a-z0-9]([a-z0-9\-]{0,61}[a-z0-9])?$").unwrap();
            re.is_match(&s)
        };
        let fn_valid = is_valid_dns_label(&s);
        prop_assert_eq!(fn_valid, regex_valid,
            "Mismatch for input {:?}: fn={}, regex={}", s, fn_valid, regex_valid);
    }

    #[test]
    fn dns_label_random_strings(s in "\\PC{0,100}") {
        // Should never panic, regardless of input
        let _ = is_valid_dns_label(&s);
    }

    #[test]
    fn config_parse_valid_yaml(
        names in prop::collection::vec("[a-z][a-z0-9]{0,8}", 1..6),
        cmds in prop::collection::vec("[a-z ]{1,20}", 1..6),
    ) {
        // Build a valid YAML config with the generated process names and commands
        let count = names.len().min(cmds.len());
        let mut yaml = String::from("processes:\n");
        for i in 0..count {
            use std::fmt::Write;
            let _ = write!(yaml, "  {}:\n    cmd: {}\n", names[i], cmds[i]);
        }
        // Parsing should not panic (may fail on duplicate keys, which is fine)
        let result: Result<agent_procs::config::ProjectConfig, _> = serde_yaml::from_str(&yaml);
        // We just ensure no panic occurred; parse errors are acceptable for random input
        let _ = result;
    }
}
