use serde::{Deserialize, Serialize};

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
}

fn default_max_steps() -> u32 { 20 }
fn default_timeout_ms() -> u64 { 30_000 }

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
        assert_eq!(spec.max_steps, 50);
        assert_eq!(spec.timeout_ms, 60_000);
    }

    #[test]
    fn invalid_yaml_returns_parse_error() {
        let yaml = "name: [unclosed";
        let err: Result<AgentSpec, _> = serde_yaml::from_str(yaml);
        assert!(err.is_err());
    }

}
