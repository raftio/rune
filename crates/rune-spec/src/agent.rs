use serde::{Deserialize, Serialize};

use crate::ModelsSpec;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Arch {
    #[default]
    ReAct,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpec {
    pub name: String,
    pub version: String,
    pub instructions: String,
    #[serde(default)]
    pub arch: Arch,
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// Key into `models.model_mapping` for the primary model (e.g. `default`, `fast`).
    #[serde(default = "default_model_alias")]
    pub default_model: String,
    #[serde(default)]
    pub models: ModelsSpec,
    /// Built-in (`rune@…`) and custom tool names; merged with tools discovered as scripts under `tools/`.
    #[serde(default, alias = "tools")]
    pub toolset: Vec<String>,
    /// Network memberships for rune-network policy (default: `bridge`).
    #[serde(default = "default_networks")]
    pub networks: Vec<String>,
    /// Remote or local skill refs (`owner/repo/skill-name`); local copies live under `skills/`.
    #[serde(default)]
    pub skills: Vec<String>,
    /// When true, OpenAI chat completions use `tool_choice: "required"` whenever tools are present (forces at least one tool call per step).
    #[serde(default)]
    pub require_tool_call: bool,
}

fn default_networks() -> Vec<String> {
    vec!["bridge".to_string()]
}

fn default_model_alias() -> String {
    "default".to_string()
}

fn default_max_steps() -> u32 {
    20
}
fn default_timeout_ms() -> u64 {
    30_000
}

impl AgentSpec {}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_yaml() -> &'static str {
        r"name: test-agent
version: 0.1.0
instructions: You are a test agent.
default_model: default
models:
  model_mapping:
    default: claude-sonnet-4-6
"
    }

    #[test]
    fn parse_minimal() {
        let spec: AgentSpec = serde_yaml::from_str(minimal_yaml()).unwrap();
        assert_eq!(spec.name, "test-agent");
        assert_eq!(spec.version, "0.1.0");
        assert_eq!(spec.instructions, "You are a test agent.");
        assert_eq!(spec.default_model, "default");
        assert_eq!(spec.models.model_mapping["default"], "claude-sonnet-4-6");
    }

    #[test]
    fn parse_tools_alias_maps_to_toolset() {
        let yaml = r#"
name: alias-agent
version: 0.1.0
instructions: Hi.
default_model: default
models:
  model_mapping:
    default: claude-sonnet-4-6
tools:
  - sum
  - rune@shell
"#;
        let spec: AgentSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.name, "alias-agent");
        assert_eq!(
            spec.toolset,
            vec!["sum".to_string(), "rune@shell".to_string()]
        );
    }

    #[test]
    fn parse_full() {
        let yaml = r#"
name: my-agent
version: 1.2.3
instructions: Do stuff.
default_model: fast
models:
  model_mapping:
    fast: claude-haiku-4-5
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
        assert_eq!(spec.max_steps, 50);
        assert_eq!(spec.timeout_ms, 60_000);
        assert_eq!(
            spec.toolset,
            vec!["rune@file-read".to_string(), "my_tool".to_string()]
        );
        assert_eq!(
            spec.networks,
            vec!["bridge".to_string(), "internal".to_string()]
        );
    }

    #[test]
    fn invalid_yaml_returns_parse_error() {
        let yaml = "name: [unclosed";
        let err: Result<AgentSpec, _> = serde_yaml::from_str(yaml);
        assert!(err.is_err());
    }
}
