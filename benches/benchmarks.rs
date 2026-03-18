use criterion::{Criterion, criterion_group, criterion_main};
use std::collections::HashMap;

use agent_procs::config::{ProcessDef, ProjectConfig};
use agent_procs::daemon::process_manager::is_valid_dns_label;
use agent_procs::protocol::{ErrorCode, Request, Response, Stream};

fn bench_config_parse(c: &mut Criterion) {
    let yaml = r"
processes:
  a:
    cmd: echo a
  b:
    cmd: echo b
    depends_on: [a]
  c:
    cmd: echo c
    depends_on: [a]
  d:
    cmd: echo d
    depends_on: [b, c]
";
    c.bench_function("config_parse", |b| {
        b.iter(|| {
            let _: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
        });
    });
}

fn bench_startup_order(c: &mut Criterion) {
    // Build a config with ~10 processes forming a dependency chain.
    let names: Vec<String> = (0..10).map(|i| format!("proc{i}")).collect();
    let mut processes = HashMap::new();
    for (i, name) in names.iter().enumerate() {
        let deps = if i == 0 {
            vec![]
        } else {
            // Each process depends on the previous one
            vec![names[i - 1].clone()]
        };
        processes.insert(
            name.clone(),
            ProcessDef {
                cmd: format!("echo {name}"),
                cwd: None,
                env: HashMap::new(),
                ready: None,
                depends_on: deps,
                port: None,
                autorestart: None,
                max_restarts: None,
                restart_delay: None,
                watch: None,
                watch_ignore: None,
            },
        );
    }
    let config = ProjectConfig {
        session: None,
        processes,
        proxy: None,
        proxy_port: None,
    };
    c.bench_function("startup_order_10_procs", |b| {
        b.iter(|| {
            let _ = config.startup_order().unwrap();
        });
    });
}

fn bench_protocol_serde(c: &mut Criterion) {
    let requests: Vec<Request> = vec![
        Request::Status,
        Request::StopAll,
        Request::Shutdown,
        Request::Stop {
            target: "web".into(),
        },
        Request::Run {
            command: "cargo run".into(),
            name: Some("api".into()),
            cwd: Some("/app".into()),
            env: Some(HashMap::from([("PORT".into(), "3000".into())])),
            port: Some(3000),
            restart: None,
            watch: None,
        },
        Request::Logs {
            target: Some("api".into()),
            tail: 100,
            follow: true,
            stderr: false,
            all: false,
            timeout_secs: Some(30),
            lines: None,
        },
    ];

    let responses: Vec<Response> = vec![
        Response::Ok {
            message: "done".into(),
        },
        Response::RunOk {
            name: "web".into(),
            id: "p1".into(),
            pid: 1234,
            port: Some(3000),
            url: Some("http://127.0.0.1:3000".into()),
        },
        Response::LogLine {
            process: "api".into(),
            stream: Stream::Stdout,
            line: "Server listening on port 3000".into(),
        },
        Response::Error {
            code: ErrorCode::General,
            message: "process not found".into(),
        },
    ];

    c.bench_function("request_serialize", |b| {
        b.iter(|| {
            for req in &requests {
                let _ = serde_json::to_string(req).unwrap();
            }
        });
    });

    c.bench_function("request_deserialize", |b| {
        let jsons: Vec<String> = requests
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect();
        b.iter(|| {
            for json in &jsons {
                let _: Request = serde_json::from_str(json).unwrap();
            }
        });
    });

    c.bench_function("response_serialize", |b| {
        b.iter(|| {
            for resp in &responses {
                let _ = serde_json::to_string(resp).unwrap();
            }
        });
    });

    c.bench_function("response_deserialize", |b| {
        let jsons: Vec<String> = responses
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect();
        b.iter(|| {
            for json in &jsons {
                let _: Response = serde_json::from_str(json).unwrap();
            }
        });
    });
}

fn bench_dns_label_validation(c: &mut Criterion) {
    let inputs = vec![
        "valid-label",
        "a",
        "abc123",
        "my-service-name",
        "",
        "-invalid",
        "invalid-",
        "UPPER",
        "a-very-long-label-that-is-exactly-sixty-three-characters-long-x",
        "a-label-that-exceeds-sixty-three-characters-and-should-be-invalid-definitely",
        "has spaces",
        "has.dots",
        "has_underscores",
    ];

    c.bench_function("dns_label_validation", |b| {
        b.iter(|| {
            for input in &inputs {
                let _ = is_valid_dns_label(input);
            }
        });
    });
}

criterion_group!(
    benches,
    bench_config_parse,
    bench_startup_order,
    bench_protocol_serde,
    bench_dns_label_validation,
);
criterion_main!(benches);
