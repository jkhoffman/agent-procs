use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_SESSION: &str = "default";

#[derive(Debug, Deserialize)]
pub struct ProjectConfig {
    pub session: Option<String>,
    pub processes: HashMap<String, ProcessDef>,
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
}

impl ProjectConfig {
    pub fn startup_order(&self) -> Result<Vec<Vec<String>>, String> {
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

        for name in self.processes.keys() {
            in_degree.entry(name.as_str()).or_insert(0);
        }
        for (name, def) in &self.processes {
            for dep in &def.depends_on {
                if !self.processes.contains_key(dep) {
                    return Err(format!("unknown dependency: {} depends on {}", name, dep));
                }
                dependents.entry(dep.as_str()).or_default().push(name.as_str());
                *in_degree.entry(name.as_str()).or_insert(0) += 1;
            }
        }

        let mut groups = Vec::new();
        let mut remaining = in_degree.clone();

        loop {
            let mut ready: Vec<String> = remaining.iter()
                .filter(|(_, &deg)| deg == 0)
                .map(|(&name, _)| name.to_string())
                .collect();

            if ready.is_empty() {
                if remaining.is_empty() { break; }
                else { return Err("dependency cycle detected".into()); }
            }

            for name in &ready {
                remaining.remove(name.as_str());
                if let Some(deps) = dependents.get(name.as_str()) {
                    for dep in deps {
                        if let Some(deg) = remaining.get_mut(dep) { *deg -= 1; }
                    }
                }
            }
            ready.sort();
            groups.push(ready);
        }
        Ok(groups)
    }
}

pub fn discover_config(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join("agent-procs.yaml");
        if candidate.exists() { return Some(candidate); }
        if !dir.pop() { return None; }
    }
}

pub fn load_config(config_path: Option<&str>) -> Result<(PathBuf, ProjectConfig), String> {
    let path = match config_path {
        Some(p) => PathBuf::from(p),
        None => discover_config(&std::env::current_dir().map_err(|e| format!("cannot get cwd: {}", e))?)
            .ok_or_else(|| "no agent-procs.yaml found".to_string())?,
    };
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read config: {}", e))?;
    let config: ProjectConfig = serde_yaml::from_str(&content)
        .map_err(|e| format!("invalid config: {}", e))?;
    Ok((path, config))
}

pub fn resolve_session<'a>(cli_session: Option<&'a str>, config_session: Option<&'a str>) -> &'a str {
    cli_session.or(config_session).unwrap_or(DEFAULT_SESSION)
}
