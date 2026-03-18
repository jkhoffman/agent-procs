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
