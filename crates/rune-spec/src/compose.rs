use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use crate::error::SpecError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeSpec {
    #[serde(default = "default_version")]
    pub version: String,
    pub project: String,
    #[serde(default, rename = "rune-env")]
    pub rune_env: HashMap<String, String>,
    pub agents: IndexMap<String, AgentEntry>,
}

fn default_version() -> String {
    "1".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    pub source: String,
    #[serde(default = "default_namespace")]
    pub namespace: String,
    #[serde(default = "default_alias")]
    pub alias: String,
    #[serde(default = "default_replicas")]
    pub replicas: i32,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

fn default_namespace() -> String {
    "dev".into()
}
fn default_alias() -> String {
    "stable".into()
}
fn default_replicas() -> i32 {
    1
}

impl ComposeSpec {
    pub fn load(path: &Path) -> Result<Self, SpecError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| SpecError::Io(path.to_path_buf(), e))?;
        Self::parse(&content, &path.display().to_string())
    }

    pub fn parse(yaml: &str, source_name: &str) -> Result<Self, SpecError> {
        let spec: Self = serde_yaml::from_str(yaml)
            .map_err(|e| SpecError::Parse(source_name.to_string(), e.to_string()))?;
        spec.validate()?;
        Ok(spec)
    }

    fn validate(&self) -> Result<(), SpecError> {
        if self.project.is_empty() {
            return Err(SpecError::Validation("project name cannot be empty".into()));
        }
        if self.agents.is_empty() {
            return Err(SpecError::Validation(
                "compose file must define at least one agent".into(),
            ));
        }

        let names: HashSet<&str> = self.agents.keys().map(|s| s.as_str()).collect();
        for (name, entry) in &self.agents {
            validate_agent_name(name)?;

            for dep in &entry.depends_on {
                if !names.contains(dep.as_str()) {
                    return Err(SpecError::Validation(format!(
                        "agent '{name}' depends on '{dep}' which is not defined",
                    )));
                }
                if dep == name {
                    return Err(SpecError::Validation(format!(
                        "agent '{name}' cannot depend on itself",
                    )));
                }
            }
        }

        if self.topological_order().is_none() {
            return Err(SpecError::Validation(
                "agent dependencies contain a cycle".into(),
            ));
        }

        Ok(())
    }

}

/// Validates that an agent name is safe for use as a Kubernetes resource name,
/// Docker container label, and filesystem path component.
/// Rules: 1-63 chars, lowercase alphanumeric or '-', must start/end with alphanumeric.
fn validate_agent_name(name: &str) -> Result<(), SpecError> {
    if name.is_empty() || name.len() > 63 {
        return Err(SpecError::Validation(format!(
            "agent name '{}' must be 1-63 characters",
            name
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(SpecError::Validation(format!(
            "agent name '{}' may only contain lowercase alphanumeric characters and '-'",
            name
        )));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(SpecError::Validation(format!(
            "agent name '{}' must start and end with an alphanumeric character",
            name
        )));
    }
    Ok(())
}

impl ComposeSpec {
    /// Returns merged env for a given agent: `rune-env` base + agent-level `env` (agent wins).
    pub fn resolved_env(&self, agent_name: &str) -> HashMap<String, String> {
        let mut merged = self.rune_env.clone();
        if let Some(entry) = self.agents.get(agent_name) {
            merged.extend(entry.env.clone());
        }
        merged
    }

    /// Returns agent names in dependency order (Kahn's algorithm).
    /// Returns `None` if the graph contains a cycle.
    pub fn topological_order(&self) -> Option<Vec<String>> {
        let names: Vec<&String> = self.agents.keys().collect();
        let idx: HashMap<&str, usize> = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();

        let n = names.len();
        let mut in_degree = vec![0usize; n];
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];

        for (name, entry) in &self.agents {
            let to = idx[name.as_str()];
            for dep in &entry.depends_on {
                let from = idx[dep.as_str()];
                adj[from].push(to);
                in_degree[to] += 1;
            }
        }

        let mut queue: VecDeque<usize> = in_degree
            .iter()
            .enumerate()
            .filter(|(_, &d)| d == 0)
            .map(|(i, _)| i)
            .collect();

        let mut order = Vec::with_capacity(n);
        while let Some(node) = queue.pop_front() {
            order.push(names[node].clone());
            for &next in &adj[node] {
                in_degree[next] -= 1;
                if in_degree[next] == 0 {
                    queue.push_back(next);
                }
            }
        }

        if order.len() == n {
            Some(order)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_yaml() -> &'static str {
        r#"
project: test-project
agents:
  alpha:
    source: ./agents/alpha
  beta:
    source: ./agents/beta
    depends_on:
      - alpha
  gamma:
    source: ./agents/gamma
    depends_on:
      - alpha
      - beta
"#
    }

    #[test]
    fn parse_valid_compose() {
        let spec = ComposeSpec::parse(valid_yaml(), "test").unwrap();
        assert_eq!(spec.project, "test-project");
        assert_eq!(spec.version, "1");
        assert_eq!(spec.agents.len(), 3);

        let alpha = &spec.agents["alpha"];
        assert_eq!(alpha.source, "./agents/alpha");
        assert_eq!(alpha.namespace, "dev");
        assert_eq!(alpha.alias, "stable");
        assert_eq!(alpha.replicas, 1);
        assert!(alpha.depends_on.is_empty());
        assert!(alpha.env.is_empty());

        let gamma = &spec.agents["gamma"];
        assert_eq!(gamma.depends_on, vec!["alpha", "beta"]);
    }

    #[test]
    fn parse_custom_defaults() {
        let yaml = r#"
project: proj
agents:
  svc:
    source: ./svc
    namespace: prod
    alias: canary
    replicas: 3
"#;
        let spec = ComposeSpec::parse(yaml, "test").unwrap();
        let svc = &spec.agents["svc"];
        assert_eq!(svc.namespace, "prod");
        assert_eq!(svc.alias, "canary");
        assert_eq!(svc.replicas, 3);
    }

    #[test]
    fn rune_env_shared_and_per_agent() {
        let yaml = r#"
project: env-test
rune-env:
  SHARED_KEY: shared_val
  OVERRIDE_ME: base
agents:
  a:
    source: ./a
    env:
      OVERRIDE_ME: agent_val
      AGENT_ONLY: only
  b:
    source: ./b
"#;
        let spec = ComposeSpec::parse(yaml, "test").unwrap();
        assert_eq!(spec.rune_env.len(), 2);
        assert_eq!(spec.rune_env["SHARED_KEY"], "shared_val");

        let env_a = spec.resolved_env("a");
        assert_eq!(env_a["SHARED_KEY"], "shared_val");
        assert_eq!(env_a["OVERRIDE_ME"], "agent_val");
        assert_eq!(env_a["AGENT_ONLY"], "only");

        let env_b = spec.resolved_env("b");
        assert_eq!(env_b["SHARED_KEY"], "shared_val");
        assert_eq!(env_b["OVERRIDE_ME"], "base");
        assert!(!env_b.contains_key("AGENT_ONLY"));
    }

    #[test]
    fn rune_env_empty_by_default() {
        let spec = ComposeSpec::parse(valid_yaml(), "test").unwrap();
        assert!(spec.rune_env.is_empty());
        let env = spec.resolved_env("alpha");
        assert!(env.is_empty());
    }

    #[test]
    fn topological_order_linear_chain() {
        let spec = ComposeSpec::parse(valid_yaml(), "test").unwrap();
        let order = spec.topological_order().unwrap();

        let pos = |name: &str| order.iter().position(|n| n == name).unwrap();
        assert!(pos("alpha") < pos("beta"));
        assert!(pos("alpha") < pos("gamma"));
        assert!(pos("beta") < pos("gamma"));
    }

    #[test]
    fn topological_order_no_deps() {
        let yaml = r#"
project: flat
agents:
  a:
    source: ./a
  b:
    source: ./b
  c:
    source: ./c
"#;
        let spec = ComposeSpec::parse(yaml, "test").unwrap();
        let order = spec.topological_order().unwrap();
        assert_eq!(order.len(), 3);
    }

    #[test]
    fn topological_order_diamond() {
        let yaml = r#"
project: diamond
agents:
  root:
    source: ./root
  left:
    source: ./left
    depends_on: [root]
  right:
    source: ./right
    depends_on: [root]
  sink:
    source: ./sink
    depends_on: [left, right]
"#;
        let spec = ComposeSpec::parse(yaml, "test").unwrap();
        let order = spec.topological_order().unwrap();
        let pos = |name: &str| order.iter().position(|n| n == name).unwrap();
        assert!(pos("root") < pos("left"));
        assert!(pos("root") < pos("right"));
        assert!(pos("left") < pos("sink"));
        assert!(pos("right") < pos("sink"));
    }

    #[test]
    fn error_empty_project() {
        let yaml = r#"
project: ""
agents:
  a:
    source: ./a
"#;
        let err = ComposeSpec::parse(yaml, "test").unwrap_err();
        assert!(err.to_string().contains("project name cannot be empty"));
    }

    #[test]
    fn error_no_agents() {
        let yaml = r#"
project: empty
agents: {}
"#;
        let err = ComposeSpec::parse(yaml, "test").unwrap_err();
        assert!(err.to_string().contains("at least one agent"));
    }

    #[test]
    fn error_unknown_dependency() {
        let yaml = r#"
project: bad
agents:
  a:
    source: ./a
    depends_on:
      - nonexistent
"#;
        let err = ComposeSpec::parse(yaml, "test").unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
        assert!(err.to_string().contains("not defined"));
    }

    #[test]
    fn error_self_dependency() {
        let yaml = r#"
project: bad
agents:
  a:
    source: ./a
    depends_on:
      - a
"#;
        let err = ComposeSpec::parse(yaml, "test").unwrap_err();
        assert!(err.to_string().contains("cannot depend on itself"));
    }

    #[test]
    fn error_cycle() {
        let yaml = r#"
project: cyclic
agents:
  a:
    source: ./a
    depends_on: [b]
  b:
    source: ./b
    depends_on: [c]
  c:
    source: ./c
    depends_on: [a]
"#;
        let err = ComposeSpec::parse(yaml, "test").unwrap_err();
        assert!(err.to_string().contains("cycle"));
    }

    #[test]
    fn error_invalid_yaml() {
        let yaml = "not: valid: yaml: [";
        let err = ComposeSpec::parse(yaml, "bad.yaml").unwrap_err();
        assert!(err.to_string().contains("Parse error"));
    }

    #[test]
    fn load_missing_file() {
        let err = ComposeSpec::load(Path::new("/nonexistent/rune-compose.yaml")).unwrap_err();
        assert!(err.to_string().contains("IO error"));
    }

    #[test]
    fn preserves_insertion_order() {
        let yaml = r#"
project: ordered
agents:
  zulu:
    source: ./z
  alpha:
    source: ./a
  mike:
    source: ./m
"#;
        let spec = ComposeSpec::parse(yaml, "test").unwrap();
        let keys: Vec<&String> = spec.agents.keys().collect();
        assert_eq!(keys, vec!["zulu", "alpha", "mike"]);
    }
}
