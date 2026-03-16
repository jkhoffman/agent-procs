use crate::cli;
use crate::config::{load_config, resolve_session};
use crate::protocol::{Request, Response};
use futures::future::join_all;

pub async fn execute(
    cli_session: Option<&str>,
    only: Option<&str>,
    config_path: Option<&str>,
) -> i32 {
    let (path, config) = match load_config(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {}", e);
            return 1;
        }
    };

    let session = resolve_session(cli_session, config.session.as_deref());

    let only_set: Option<Vec<&str>> = only.map(|s| s.split(',').collect());

    let groups = match config.startup_order() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: {}", e);
            return 1;
        }
    };

    for group in &groups {
        let names: Vec<&String> = group
            .iter()
            .filter(|name| {
                only_set
                    .as_ref()
                    .is_none_or(|only| only.contains(&name.as_str()))
            })
            .collect();

        // Start all processes in this group concurrently
        let start_futures: Vec<_> = names
            .iter()
            .map(|name| {
                let def = &config.processes[*name];

                let resolved_cwd = def.cwd.as_ref().map(|c| {
                    let p = std::path::Path::new(c);
                    if p.is_relative() {
                        path.parent()
                            .unwrap_or(std::path::Path::new("."))
                            .join(p)
                            .to_string_lossy()
                            .to_string()
                    } else {
                        c.clone()
                    }
                });

                let env = if def.env.is_empty() {
                    None
                } else {
                    Some(def.env.clone())
                };

                let req = Request::Run {
                    command: def.cmd.clone(),
                    name: Some((*name).clone()),
                    cwd: resolved_cwd,
                    env,
                    port: None,
                };
                let name = (*name).clone();
                async move {
                    let result = cli::request(session, &req, true).await;
                    (name, result)
                }
            })
            .collect();

        let results = join_all(start_futures).await;

        for (name, result) in &results {
            match result {
                Ok(Response::RunOk { name, id, pid, .. }) => {
                    println!("started {} (id: {}, pid: {})", name, id, pid);
                }
                Ok(Response::Error { code, message }) => {
                    eprintln!("error starting {}: {}", name, message);
                    return *code;
                }
                _ => return 1,
            }
        }

        // Wait for ready patterns concurrently within the group
        let ready_futures: Vec<_> = names
            .iter()
            .filter_map(|name| {
                let def = &config.processes[*name];
                def.ready.as_ref().map(|ready| {
                    let req = Request::Wait {
                        target: (*name).clone(),
                        until: Some(ready.clone()),
                        regex: false,
                        exit: false,
                        timeout_secs: Some(30),
                    };
                    let name = (*name).clone();
                    async move {
                        let result = cli::request(session, &req, false).await;
                        (name, result)
                    }
                })
            })
            .collect();

        let ready_results = join_all(ready_futures).await;

        for (name, result) in &ready_results {
            match result {
                Ok(Response::WaitMatch { .. }) => println!("{} is ready", name),
                Ok(Response::WaitTimeout) => {
                    eprintln!("warning: {} did not become ready within 30s", name);
                }
                Ok(Response::Error { message, .. }) => {
                    eprintln!("error waiting for {}: {}", name, message);
                    return 1;
                }
                _ => {}
            }
        }
    }

    println!("all processes started");
    0
}
