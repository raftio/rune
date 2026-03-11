use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::SpecError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub runtime: ToolRuntime,
    /// Path to the module file (wasm or executable), relative to agent dir.
    #[serde(default)]
    pub module: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub retry_policy: RetryPolicy,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub input_schema_ref: Option<String>,
    #[serde(default)]
    pub output_schema_ref: Option<String>,
    /// A2A endpoint URL for agent tools (runtime: agent).
    /// Use `local://agent-name` for agents in the same runtime,
    /// or a full URL like `http://host:port/a2a/agent-name`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_ref: Option<String>,
    /// Max recursion depth for agent-to-agent calls (default: 5).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_depth: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolRuntime {
    Wasm,
    #[default]
    Process,
    Container,
    Agent,
    Builtin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self { Self { max_attempts: 1 } }
}

fn default_timeout_ms() -> u64 { 5_000 }
fn default_max_attempts() -> u32 { 1 }
fn default_version() -> String { "0.1.0".into() }

impl ToolDescriptor {
    pub fn is_builtin(&self) -> bool {
        self.name.starts_with("rune@")
    }

    /// Create a synthetic descriptor for a built-in tool.
    pub fn builtin(name: &str) -> Self {
        Self {
            name: name.to_string(),
            version: "0.1.0".into(),
            runtime: ToolRuntime::Builtin,
            module: String::new(),
            timeout_ms: 30_000,
            retry_policy: RetryPolicy::default(),
            capabilities: vec![],
            input_schema_ref: None,
            output_schema_ref: None,
            agent_ref: None,
            max_depth: None,
        }
    }

    pub fn load(path: &Path) -> Result<Self, SpecError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| SpecError::Io(path.to_path_buf(), e))?;
        serde_yaml::from_str(&content)
            .map_err(|e| SpecError::Parse(path.display().to_string(), e.to_string()))
    }

    /// Load all `*.yaml` tool descriptors from a directory.
    pub fn load_dir(dir: &Path) -> Result<Vec<Self>, SpecError> {
        let mut tools = vec![];
        for entry in std::fs::read_dir(dir)
            .map_err(|e| SpecError::Io(dir.to_path_buf(), e))?
        {
            let entry = entry.map_err(|e| SpecError::Io(dir.to_path_buf(), e))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                tools.push(Self::load(&path)?);
            }
        }
        Ok(tools)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_uses_defaults() {
        let tool: ToolDescriptor = serde_yaml::from_str("name: my_tool\n").unwrap();
        assert_eq!(tool.name, "my_tool");
        assert_eq!(tool.version, "0.1.0");
        assert!(matches!(tool.runtime, ToolRuntime::Process));
        assert!(tool.module.is_empty());
        assert_eq!(tool.timeout_ms, 5_000);
        assert_eq!(tool.retry_policy.max_attempts, 1);
        assert!(tool.capabilities.is_empty());
        assert!(tool.input_schema_ref.is_none());
        assert!(tool.output_schema_ref.is_none());
        assert!(tool.agent_ref.is_none());
        assert!(tool.max_depth.is_none());
    }

    #[test]
    fn parse_full_process_tool() {
        let yaml = r#"
name: search_kb
version: 1.0.0
runtime: process
module: tools/search_kb.py
timeout_ms: 8000
retry_policy:
  max_attempts: 3
capabilities:
  - filesystem
  - network
input_schema_ref: schemas/search_input.json
output_schema_ref: schemas/search_output.json
"#;
        let tool: ToolDescriptor = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(tool.name, "search_kb");
        assert_eq!(tool.version, "1.0.0");
        assert!(matches!(tool.runtime, ToolRuntime::Process));
        assert_eq!(tool.module, "tools/search_kb.py");
        assert_eq!(tool.timeout_ms, 8_000);
        assert_eq!(tool.retry_policy.max_attempts, 3);
        assert_eq!(tool.capabilities, vec!["filesystem", "network"]);
        assert_eq!(tool.input_schema_ref.as_deref(), Some("schemas/search_input.json"));
        assert_eq!(tool.output_schema_ref.as_deref(), Some("schemas/search_output.json"));
    }

    #[test]
    fn parse_agent_tool() {
        let yaml = r#"
name: delegate
runtime: agent
agent_ref: local://worker-agent
max_depth: 3
timeout_ms: 30000
"#;
        let tool: ToolDescriptor = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(tool.runtime, ToolRuntime::Agent));
        assert_eq!(tool.agent_ref.as_deref(), Some("local://worker-agent"));
        assert_eq!(tool.max_depth, Some(3));
        assert_eq!(tool.timeout_ms, 30_000);
    }

    #[test]
    fn parse_wasm_tool() {
        let yaml = "name: wasm_tool\nruntime: wasm\nmodule: tools/module.wasm\n";
        let tool: ToolDescriptor = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(tool.runtime, ToolRuntime::Wasm));
        assert_eq!(tool.module, "tools/module.wasm");
    }

    #[test]
    fn parse_container_tool() {
        let yaml = "name: container_tool\nruntime: container\n";
        let tool: ToolDescriptor = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(tool.runtime, ToolRuntime::Container));
    }

    #[test]
    fn is_builtin_returns_true_for_rune_prefix() {
        let tool: ToolDescriptor = serde_yaml::from_str("name: rune@file-read\n").unwrap();
        assert!(tool.is_builtin());
    }

    #[test]
    fn is_builtin_returns_false_for_custom() {
        let tool: ToolDescriptor = serde_yaml::from_str("name: my_custom_tool\n").unwrap();
        assert!(!tool.is_builtin());
    }

    #[test]
    fn builtin_constructor_sets_correct_fields() {
        let tool = ToolDescriptor::builtin("rune@web-search");
        assert_eq!(tool.name, "rune@web-search");
        assert!(matches!(tool.runtime, ToolRuntime::Builtin));
        assert!(tool.is_builtin());
        assert_eq!(tool.timeout_ms, 30_000);
        assert_eq!(tool.version, "0.1.0");
        assert!(tool.module.is_empty());
        assert!(tool.agent_ref.is_none());
        assert!(tool.max_depth.is_none());
    }

    #[test]
    fn retry_policy_default_max_attempts() {
        let tool: ToolDescriptor = serde_yaml::from_str("name: t\n").unwrap();
        assert_eq!(tool.retry_policy.max_attempts, 1);
    }

    #[test]
    fn runtime_variants_all_parse() {
        for (val, variant) in &[
            ("wasm", "Wasm"),
            ("process", "Process"),
            ("container", "Container"),
            ("agent", "Agent"),
            ("builtin", "Builtin"),
        ] {
            let yaml = format!("name: t\nruntime: {val}\n");
            let tool: ToolDescriptor = serde_yaml::from_str(&yaml).unwrap();
            assert!(format!("{:?}", tool.runtime).contains(variant));
        }
    }

    #[test]
    fn load_missing_file_returns_io_error() {
        let err = ToolDescriptor::load(Path::new("/nonexistent/tool.yaml")).unwrap_err();
        assert!(err.to_string().contains("IO error"));
    }

    #[test]
    fn load_dir_missing_dir_returns_io_error() {
        let err = ToolDescriptor::load_dir(Path::new("/nonexistent/tools")).unwrap_err();
        assert!(err.to_string().contains("IO error"));
    }

    #[test]
    fn load_dir_loads_yaml_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let tool_yaml = "name: tool_a\ntimeout_ms: 1000\n";
        std::fs::write(dir.path().join("tool_a.yaml"), tool_yaml).unwrap();
        std::fs::write(dir.path().join("README.md"), "docs").unwrap();
        std::fs::write(dir.path().join("config.toml"), "key = val").unwrap();

        let tools = ToolDescriptor::load_dir(dir.path()).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "tool_a");
    }

    #[test]
    fn load_dir_loads_multiple_tools() {
        let dir = tempfile::tempdir().unwrap();
        for name in &["alpha", "beta", "gamma"] {
            std::fs::write(
                dir.path().join(format!("{name}.yaml")),
                format!("name: {name}\n"),
            )
            .unwrap();
        }

        let mut tools = ToolDescriptor::load_dir(dir.path()).unwrap();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0].name, "alpha");
        assert_eq!(tools[1].name, "beta");
        assert_eq!(tools[2].name, "gamma");
    }

    #[test]
    fn load_dir_empty_dir_returns_empty_vec() {
        let dir = tempfile::tempdir().unwrap();
        let tools = ToolDescriptor::load_dir(dir.path()).unwrap();
        assert!(tools.is_empty());
    }
}
