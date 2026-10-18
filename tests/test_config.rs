use agent_procs::config::*;
use std::io::Write;
use tempfile::TempDir;

#[test]
fn test_parse_minimal_config() {
    let yaml = "processes:\n  web:\n    cmd: npm run dev\n";
    let config: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.processes.len(), 1);
    assert_eq!(config.processes["web"].cmd, "npm run dev");
}

#[test]
fn test_parse_full_config() {
    let yaml = r#"
processes:
  db:
    cmd: docker compose up postgres
    ready: "ready to accept connections"
  api:
    cmd: ./start-api-server
    cwd: ./backend
    env:
      DATABASE_URL: postgres://localhost:5432/mydb
    ready: "Listening on :8080"
    depends_on: [db]
"#;
    let config: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.processes["api"].cwd, Some("./backend".into()));
    assert_eq!(
        config.processes["api"].env.get("DATABASE_URL").unwrap(),
        "postgres://localhost:5432/mydb"
    );
    assert_eq!(config.processes["api"].depends_on, vec!["db"]);
}

#[test]
fn test_topological_sort_concurrent_group() {
    let yaml = r"
processes:
  db:
    cmd: start-db
  cache:
    cmd: start-cache
  api:
    cmd: start-api
    depends_on: [db, cache]
";
    let config: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
    let order = config.startup_order().unwrap();
    assert_eq!(order.len(), 2);
    assert!(order[0].contains(&"db".to_string()));
    assert!(order[0].contains(&"cache".to_string()));
    assert_eq!(order[0].len(), 2);
    assert_eq!(order[1], vec!["api"]);
}

#[test]
fn test_topological_sort_cycle_detection() {
    let yaml = "processes:\n  a:\n    cmd: x\n    depends_on: [b]\n  b:\n    cmd: y\n    depends_on: [a]\n";
    let config: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(config.startup_order().is_err());
}

#[test]
fn test_discover_config_walks_up() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("agent-procs.yaml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    writeln!(f, "processes:\n  web:\n    cmd: npm run dev").unwrap();
    let subdir = tmp.path().join("src/deep");
    std::fs::create_dir_all(&subdir).unwrap();
    assert_eq!(discover_config(&subdir), Some(config_path));
}

#[test]
fn test_config_with_proxy_fields() {
    let yaml = r"
proxy: true
proxy_port: 9000
processes:
  web:
    cmd: npm run dev
    port: 3000
";
    let config: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.proxy, Some(true));
    assert_eq!(config.proxy_port, Some(9000));
    assert_eq!(config.processes["web"].port, Some(3000));
}

#[test]
fn test_config_proxy_fields_backward_compat() {
    // Old configs without proxy fields should still parse
    let yaml = "processes:\n  web:\n    cmd: npm run dev\n";
    let config: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.proxy, None);
    assert_eq!(config.proxy_port, None);
    assert_eq!(config.processes["web"].port, None);
}

#[test]
fn test_parse_config_with_restart_and_watch() {
    let yaml = r#"
processes:
  server:
    cmd: "npm start"
    autorestart: on-failure
    max_restarts: 5
    restart_delay: 2000
    watch:
      - "src/**"
      - "config/*"
    watch_ignore:
      - "*.generated.ts"
"#;
    let config: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
    let server = &config.processes["server"];
    assert_eq!(server.autorestart.as_deref(), Some("on-failure"));
    assert_eq!(server.max_restarts, Some(5));
    assert_eq!(server.restart_delay, Some(2000));
    assert_eq!(server.watch.as_ref().unwrap().len(), 2);
    assert_eq!(server.watch_ignore.as_ref().unwrap().len(), 1);
}

#[test]
fn test_parse_config_backward_compat_no_restart_fields() {
    let yaml = r#"
processes:
  db:
    cmd: "pg_ctl start"
"#;
    let config: ProjectConfig = serde_yaml::from_str(yaml).unwrap();
    let db = &config.processes["db"];
    assert!(db.autorestart.is_none());
    assert!(db.max_restarts.is_none());
    assert!(db.watch.is_none());
}

#[test]
fn test_discover_config_returns_none() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(discover_config(tmp.path()), None);
}

// YAML parsing contract. These go through `load_config` so they do not depend
// on which YAML crate is used; they pin what configs are accepted and how
// errors are reported.

fn load(yaml: &str) -> Result<ProjectConfig, agent_procs::error::ConfigError> {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("agent-procs.yaml");
    std::fs::write(&path, yaml).unwrap();
    load_config(Some(path.to_str().unwrap())).map(|(_, config)| config)
}

#[test]
fn test_yaml_readme_example_parses() {
    let config = load(
        r#"
session: myproject
proxy: true
proxy_port: 9095
processes:
  db:
    cmd: docker compose up postgres
    ready: "ready to accept connections"
    autorestart: always
    max_restarts: 3
  api:
    cmd: ./start-api-server
    cwd: ./backend
    env:
      DATABASE_URL: postgres://localhost:5432/mydb
    ready: "Listening on :8080"
    port: 8080
    depends_on: [db]
    autorestart: on-failure
    restart_delay: 250
    watch:
      - "src/**"
      - "config/*"
    watch_ignore:
      - "*.generated.ts"
"#,
    )
    .unwrap();
    assert_eq!(config.session.as_deref(), Some("myproject"));
    assert_eq!(config.proxy, Some(true));
    assert_eq!(config.proxy_port, Some(9095));
    let api = &config.processes["api"];
    assert_eq!(api.port, Some(8080));
    assert_eq!(api.depends_on, vec!["db"]);
    assert_eq!(api.restart_delay, Some(250));
    assert_eq!(api.watch.as_deref().unwrap().len(), 2);
    assert_eq!(config.processes["db"].max_restarts, Some(3));
}

#[test]
fn test_yaml_unquoted_scalars_load_as_strings() {
    let config = load(
        "session: 2024\nprocesses:\n  a:\n    cmd: x\n    env:\n      PORT: 3000\n      DEBUG: true\n      RATE: 1.5\n",
    )
    .unwrap();
    assert_eq!(config.session.as_deref(), Some("2024"));
    let env = &config.processes["a"].env;
    assert_eq!(env["PORT"], "3000");
    assert_eq!(env["DEBUG"], "true");
    assert_eq!(env["RATE"], "1.5");
}

#[test]
fn test_yaml_bool_spellings() {
    for ok in ["true", "True", "TRUE", "false"] {
        assert!(
            load(&format!("proxy: {ok}\nprocesses: {{}}\n")).is_ok(),
            "{ok} should parse"
        );
    }
    for bad in ["yes", "on", "\"true\""] {
        assert!(
            load(&format!("proxy: {bad}\nprocesses: {{}}\n")).is_err(),
            "{bad} should be rejected"
        );
    }
}

#[test]
fn test_yaml_null_and_missing_optionals() {
    for cwd in ["cwd: ~", "cwd:", "cwd: null"] {
        let config = load(&format!("processes:\n  a:\n    cmd: x\n    {cwd}\n")).unwrap();
        assert_eq!(config.processes["a"].cwd, None, "{cwd}");
    }
}

#[test]
fn test_yaml_unknown_fields_are_ignored() {
    let config = load("extra: 1\nprocesses:\n  a:\n    cmd: x\n    bogus: [1, 2]\n").unwrap();
    assert_eq!(config.processes["a"].cmd, "x");
}

#[test]
fn test_yaml_duplicate_keys() {
    // A repeated process name silently keeps the last definition.
    let config = load("processes:\n  a:\n    cmd: first\n  a:\n    cmd: second\n").unwrap();
    assert_eq!(config.processes["a"].cmd, "second");
    let err = load("processes:\n  a:\n    cmd: x\n    cmd: y\n").unwrap_err();
    assert!(err.to_string().contains("duplicate field"), "{err}");
}

#[test]
fn test_yaml_anchors_work_but_merge_keys_do_not() {
    let config = load(
        "processes:\n  a:\n    cmd: x\n    env: &e\n      K: v\n  b:\n    cmd: y\n    env: *e\n",
    )
    .unwrap();
    assert_eq!(config.processes["b"].env["K"], "v");

    let err = load("base: &base\n  cmd: x\nprocesses:\n  a:\n    <<: *base\n    ready: up\n")
        .unwrap_err();
    assert!(err.to_string().contains("missing field `cmd`"), "{err}");
}

#[test]
fn test_yaml_document_markers_and_crlf() {
    assert!(load("---\nprocesses:\n  a:\n    cmd: x\n...\n").is_ok());
    let config = load("processes:\r\n  a:\r\n    cmd: x\r\n").unwrap();
    assert_eq!(config.processes["a"].cmd, "x");
    assert!(load("---\nprocesses: {}\n---\nprocesses: {}\n").is_err());
}

#[test]
fn test_yaml_port_range_enforced() {
    for bad in ["65536", "-1", "abc"] {
        assert!(
            load(&format!("processes:\n  a:\n    cmd: x\n    port: {bad}\n")).is_err(),
            "port {bad}"
        );
    }
}

#[test]
fn test_yaml_error_messages() {
    let missing = load("processes:\n  a:\n    ready: up\n")
        .unwrap_err()
        .to_string();
    assert_eq!(
        missing,
        "invalid config: processes.a: missing field `cmd` at line 3 column 5"
    );
    let bad_port = load("processes:\n  a:\n    cmd: x\n    port: abc\n")
        .unwrap_err()
        .to_string();
    assert_eq!(
        bad_port,
        "invalid config: processes.a.port: invalid type: string \"abc\", expected u16 at line 4 column 11"
    );
    let syntax = load("processes: [unclosed\n").unwrap_err().to_string();
    assert!(syntax.starts_with("invalid config: "), "{syntax}");
}

#[test]
fn test_yaml_block_scalar_cmd() {
    let config = load("processes:\n  a:\n    cmd: |\n      echo one\n      echo two\n").unwrap();
    assert_eq!(config.processes["a"].cmd, "echo one\necho two\n");
    let config = load("processes:\n  a:\n    cmd: >-\n      echo one\n      echo two\n").unwrap();
    assert_eq!(config.processes["a"].cmd, "echo one echo two");
}

#[test]
fn test_yaml_alias_expansion_bomb_does_not_hang() {
    let mut lines = vec!["a: &a [x, x]".to_string()];
    let mut prev = 'a';
    for next in 'b'..='l' {
        let refs = vec![format!("*{prev}"); 9].join(", ");
        lines.push(format!("{next}: &{next} [{refs}]"));
        prev = next;
    }
    lines.push("processes:\n  p:\n    cmd: x".to_string());
    assert!(load(&lines.join("\n")).is_ok());
}
