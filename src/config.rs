//! YAML configuration file parsing and dependency-ordered startup.
//!
//! An `agent-procs.yaml` file declares a set of [`ProcessDef`]s, optional
//! session name, and proxy settings.  [`ProjectConfig::startup_order`]
//! topologically sorts processes by `depends_on` edges so they can be
//! launched in the correct order.
//!
//! Use [`load_config`] to read and parse the file, or [`discover_config`]
//! to walk up from the current directory until one is found.

use crate::error::ConfigError;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_SESSION: &str = "default";

#[derive(Debug, Deserialize)]
pub struct ProjectConfig {
    pub session: Option<String>,
    pub processes: HashMap<String, ProcessDef>,
    #[serde(default)]
    pub proxy: Option<bool>,
    #[serde(default)]
    pub proxy_port: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub struct ProcessDef {
    pub cmd: String,
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub ready: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub port: Option<u16>,
}

impl ProjectConfig {
    /// Compute the startup order by topologically sorting processes.
    ///
    /// Returns groups of process names; processes within a group are
    /// independent and can start concurrently.
    ///
    /// # Examples
    ///
    /// ```
    /// use agent_procs::config::{ProjectConfig, ProcessDef};
    /// use std::collections::HashMap;
    ///
    /// let config = ProjectConfig {
    ///     session: None,
    ///     processes: HashMap::from([
    ///         ("db".into(), ProcessDef {
    ///             cmd: "pg".into(), cwd: None, env: HashMap::new(),
    ///             ready: None, depends_on: vec![], port: None,
    ///         }),
    ///         ("api".into(), ProcessDef {
    ///             cmd: "node".into(), cwd: None, env: HashMap::new(),
    ///             ready: None, depends_on: vec!["db".into()], port: None,
    ///         }),
    ///     ]),
    ///     proxy: None,
    ///     proxy_port: None,
    /// };
    /// let groups = config.startup_order().unwrap();
    /// assert_eq!(groups.len(), 2);
    /// assert_eq!(groups[0], vec!["db"]);
    /// assert_eq!(groups[1], vec!["api"]);
    /// ```
    #[must_use = "startup order should be used to launch processes"]
    pub fn startup_order(&self) -> Result<Vec<Vec<String>>, ConfigError> {
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

        for name in self.processes.keys() {
            in_degree.entry(name.as_str()).or_insert(0);
        }
        for (name, def) in &self.processes {
            for dep in &def.depends_on {
                if !self.processes.contains_key(dep) {
                    return Err(ConfigError::UnknownDep {
                        from: name.clone(),
                        to: dep.clone(),
                    });
                }
                dependents
                    .entry(dep.as_str())
                    .or_default()
                    .push(name.as_str());
                *in_degree.entry(name.as_str()).or_insert(0) += 1;
            }
        }

        let mut groups = Vec::new();
        let mut remaining = in_degree.clone();

        loop {
            let mut ready: Vec<String> = remaining
                .iter()
                .filter(|(_, deg)| **deg == 0)
                .map(|(name, _)| (*name).to_string())
                .collect();

            if ready.is_empty() {
                if remaining.is_empty() {
                    break;
                }
                return Err(ConfigError::CycleDetected);
            }

            for name in &ready {
                remaining.remove(name.as_str());
                if let Some(deps) = dependents.get(name.as_str()) {
                    for dep in deps {
                        if let Some(deg) = remaining.get_mut(dep) {
                            *deg -= 1;
                        }
                    }
                }
            }
            ready.sort();
            groups.push(ready);
        }
        Ok(groups)
    }
}

#[must_use]
pub fn discover_config(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join("agent-procs.yaml");
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

#[must_use = "config should be used after loading"]
pub fn load_config(config_path: Option<&str>) -> Result<(PathBuf, ProjectConfig), ConfigError> {
    let path = match config_path {
        Some(p) => PathBuf::from(p),
        None => discover_config(&std::env::current_dir().map_err(ConfigError::Cwd)?)
            .ok_or(ConfigError::NotFound)?,
    };
    let content = std::fs::read_to_string(&path).map_err(ConfigError::Read)?;
    let config: ProjectConfig = serde_yaml::from_str(&content).map_err(ConfigError::Parse)?;
    Ok((path, config))
}

pub fn resolve_session<'a>(
    cli_session: Option<&'a str>,
    config_session: Option<&'a str>,
) -> &'a str {
    cli_session.or(config_session).unwrap_or(DEFAULT_SESSION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ConfigError;
    use std::collections::HashMap;

    fn proc_def(cmd: &str, depends_on: Vec<&str>) -> ProcessDef {
        ProcessDef {
            cmd: cmd.into(),
            cwd: None,
            env: HashMap::new(),
            ready: None,
            depends_on: depends_on.into_iter().map(String::from).collect(),
            port: None,
        }
    }

    fn config_with(procs: Vec<(&str, Vec<&str>)>) -> ProjectConfig {
        ProjectConfig {
            session: None,
            processes: procs
                .into_iter()
                .map(|(name, deps)| (name.into(), proc_def("true", deps)))
                .collect(),
            proxy: None,
            proxy_port: None,
        }
    }

    #[test]
    fn test_startup_order_no_deps() {
        let cfg = config_with(vec![("a", vec![]), ("b", vec![]), ("c", vec![])]);
        let groups = cfg.startup_order().unwrap();
        assert_eq!(groups.len(), 1);
        let mut names = groups[0].clone();
        names.sort();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_startup_order_linear_chain() {
        // a depends on nothing, b depends on a, c depends on b
        let cfg = config_with(vec![("a", vec![]), ("b", vec!["a"]), ("c", vec!["b"])]);
        let groups = cfg.startup_order().unwrap();
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0], vec!["a"]);
        assert_eq!(groups[1], vec!["b"]);
        assert_eq!(groups[2], vec!["c"]);
    }

    #[test]
    fn test_startup_order_diamond() {
        // d depends on b and c; b and c depend on a
        let cfg = config_with(vec![
            ("a", vec![]),
            ("b", vec!["a"]),
            ("c", vec!["a"]),
            ("d", vec!["b", "c"]),
        ]);
        let groups = cfg.startup_order().unwrap();
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0], vec!["a"]);
        let mut g1 = groups[1].clone();
        g1.sort();
        assert_eq!(g1, vec!["b", "c"]);
        assert_eq!(groups[2], vec!["d"]);
    }

    #[test]
    fn test_startup_order_cycle_detected() {
        let cfg = config_with(vec![("a", vec!["b"]), ("b", vec!["a"])]);
        let err = cfg.startup_order().unwrap_err();
        assert!(matches!(err, ConfigError::CycleDetected));
    }

    #[test]
    fn test_startup_order_unknown_dep() {
        let cfg = config_with(vec![("a", vec!["nonexistent"])]);
        let err = cfg.startup_order().unwrap_err();
        assert!(matches!(err, ConfigError::UnknownDep { .. }));
    }

    #[test]
    fn test_startup_order_empty() {
        let cfg = config_with(vec![]);
        let groups = cfg.startup_order().unwrap();
        assert!(groups.is_empty());
    }

    #[test]
    fn test_discover_config_finds_file() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("agent-procs.yaml");
        std::fs::write(&config_path, "processes: {}").unwrap();
        let found = discover_config(tmp.path());
        assert_eq!(found, Some(config_path));
    }

    #[test]
    fn test_discover_config_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let found = discover_config(tmp.path());
        assert!(found.is_none());
    }

    #[test]
    fn test_load_config_parse_error() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("bad.yaml");
        std::fs::write(&config_path, "{{{{not valid yaml at all").unwrap();
        let err = load_config(Some(config_path.to_str().unwrap())).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));
    }

    #[test]
    fn test_resolve_session_priority() {
        // cli_session takes highest priority
        assert_eq!(resolve_session(Some("cli"), Some("config")), "cli");
        // config_session if no cli
        assert_eq!(resolve_session(None, Some("config")), "config");
        // DEFAULT_SESSION as fallback
        assert_eq!(resolve_session(None, None), DEFAULT_SESSION);
    }
}
