use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpec {
    pub name: String,
    pub version: String,
    pub instructions: String,
    pub default_model: String,
    #[serde(default)]
    pub toolset: Vec<String>,
    #[serde(default)]
    pub memory_profile: MemoryProfile,
    #[serde(default)]
    pub routing_hints: HashMap<String, serde_json::Value>,
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// rune-network memberships. Agents can only call each other when they
    /// share at least one network. Defaults to ["bridge"].
    #[serde(default = "default_networks")]
    pub networks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryProfile {
    #[default]
    Minimal,
    Standard,
    Extended,
}

fn default_max_steps() -> u32 { 20 }
fn default_timeout_ms() -> u64 { 30_000 }
fn default_networks() -> Vec<String> { vec!["bridge".into()] }

impl AgentSpec {}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_yaml() -> &'static str {
        "name: test-agent\nversion: 0.1.0\ninstructions: You are a test agent.\ndefault_model: default\n"
    }

    #[test]
    fn parse_minimal() {
        let spec: AgentSpec = serde_yaml::from_str(minimal_yaml()).unwrap();
        assert_eq!(spec.name, "test-agent");
        assert_eq!(spec.version, "0.1.0");
        assert_eq!(spec.instructions, "You are a test agent.");
        assert_eq!(spec.default_model, "default");
    }

    #[test]
    fn defaults_applied() {
        let spec: AgentSpec = serde_yaml::from_str(minimal_yaml()).unwrap();
        assert!(spec.toolset.is_empty());
        assert!(matches!(spec.memory_profile, MemoryProfile::Minimal));
        assert!(spec.routing_hints.is_empty());
        assert_eq!(spec.max_steps, 20);
        assert_eq!(spec.timeout_ms, 30_000);
        assert_eq!(spec.networks, vec!["bridge"]);
    }

    #[test]
    fn parse_full() {
        let yaml = r#"
name: my-agent
version: 1.2.3
instructions: Do stuff.
default_model: fast
toolset:
  - rune@file-read
  - my_tool
memory_profile: extended
max_steps: 50
timeout_ms: 60000
networks:
  - bridge
  - internal
routing_hints:
  priority: 1
"#;
        let spec: AgentSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.name, "my-agent");
        assert_eq!(spec.version, "1.2.3");
        assert_eq!(spec.default_model, "fast");
        assert_eq!(spec.toolset, vec!["rune@file-read", "my_tool"]);
        assert!(matches!(spec.memory_profile, MemoryProfile::Extended));
        assert_eq!(spec.max_steps, 50);
        assert_eq!(spec.timeout_ms, 60_000);
        assert_eq!(spec.networks, vec!["bridge", "internal"]);
        assert!(spec.routing_hints.contains_key("priority"));
    }

    #[test]
    fn memory_profile_variants() {
        for (yaml_val, expected) in &[
            ("minimal", "Minimal"),
            ("standard", "Standard"),
            ("extended", "Extended"),
        ] {
            let yaml = format!(
                "name: a\nversion: 0.1.0\ninstructions: x\ndefault_model: d\nmemory_profile: {yaml_val}\n"
            );
            let spec: AgentSpec = serde_yaml::from_str(&yaml).unwrap();
            assert!(format!("{:?}", spec.memory_profile).contains(expected));
        }
    }

    #[test]
    fn memory_profile_default_is_minimal() {
        let spec: AgentSpec = serde_yaml::from_str(minimal_yaml()).unwrap();
        assert!(matches!(spec.memory_profile, MemoryProfile::Minimal));
    }

    #[test]
    fn toolset_with_builtin_and_custom() {
        let yaml = r#"
name: a
version: 0.1.0
instructions: x
default_model: d
toolset:
  - rune@shell
  - rune@web-search
  - custom_tool
"#;
        let spec: AgentSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.toolset.len(), 3);
        assert!(spec.toolset[0].starts_with("rune@"));
        assert_eq!(spec.toolset[2], "custom_tool");
    }

    #[test]
    fn networks_default_contains_bridge() {
        let spec: AgentSpec = serde_yaml::from_str(minimal_yaml()).unwrap();
        assert_eq!(spec.networks, vec!["bridge"]);
    }

    #[test]
    fn networks_custom() {
        let yaml = "name: a\nversion: 0.1.0\ninstructions: x\ndefault_model: d\nnetworks: [net-a, net-b]\n";
        let spec: AgentSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.networks, vec!["net-a", "net-b"]);
    }

    #[test]
    fn invalid_yaml_returns_parse_error() {
        let yaml = "name: [unclosed";
        let err: Result<AgentSpec, _> = serde_yaml::from_str(yaml);
        assert!(err.is_err());
    }
}
