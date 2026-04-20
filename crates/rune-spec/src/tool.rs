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
    /// MCP server name (from `mcp_servers:`) this tool belongs to.
    /// Required when `runtime: mcp`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mcp_server: Option<String>,
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
    Mcp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_attempts: 1 }
    }
}

fn default_timeout_ms() -> u64 {
    5_000
}
fn default_max_attempts() -> u32 {
    1
}
fn default_version() -> String {
    "0.1.0".into()
}

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
            mcp_server: None,
        }
    }

    pub fn load(path: &Path) -> Result<Self, SpecError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| SpecError::Io(path.to_path_buf(), e))?;
        serde_yaml::from_str(&content)
            .map_err(|e| SpecError::Parse(path.display().to_string(), e.to_string()))
    }

    /// Extensions supported for process tools (must match `rune-runtime` process runner interpreters).
    pub const PROCESS_SCRIPT_EXTENSIONS: [&str; 4] = ["py", "js", "mjs", "ts"];

    fn is_process_script_path(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| {
                Self::PROCESS_SCRIPT_EXTENSIONS
                    .iter()
                    .any(|ext| ext.eq_ignore_ascii_case(e))
            })
            .unwrap_or(false)
    }

    /// Build a process-tool descriptor from a script file under `agent_dir` (e.g. `tools/sum.py`).
    pub fn for_process_script_file(agent_dir: &Path, script_path: &Path) -> Result<Self, SpecError> {
        if !script_path.is_file() {
            return Err(SpecError::Validation(format!(
                "not a file: {}",
                script_path.display()
            )));
        }
        if !Self::is_process_script_path(script_path) {
            return Err(SpecError::Validation(format!(
                "unsupported tool script extension: {}",
                script_path.display()
            )));
        }
        let rel = script_path.strip_prefix(agent_dir).map_err(|_| {
            SpecError::Validation(format!(
                "script {} is not under agent directory {}",
                script_path.display(),
                agent_dir.display()
            ))
        })?;
        let module = rel.to_string_lossy().replace('\\', "/");
        let name = script_path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                SpecError::Validation(format!("invalid tool script name: {}", script_path.display()))
            })?
            .to_string();

        Ok(Self {
            name,
            version: default_version(),
            runtime: ToolRuntime::Process,
            module,
            timeout_ms: default_timeout_ms(),
            retry_policy: RetryPolicy::default(),
            capabilities: vec![],
            input_schema_ref: None,
            output_schema_ref: None,
            agent_ref: None,
            max_depth: None,
            mcp_server: None,
        })
    }

    /// Discover `tools/*.py`, `tools/*.js`, `tools/*.mjs`, `tools/*.ts` and build process tool descriptors.
    pub fn discover_process_scripts(agent_dir: &Path) -> Result<Vec<Self>, SpecError> {
        let dir = agent_dir.join("tools");
        if !dir.is_dir() {
            return Ok(vec![]);
        }

        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        for entry in std::fs::read_dir(&dir).map_err(|e| SpecError::Io(dir.clone(), e))? {
            let entry = entry.map_err(|e| SpecError::Io(dir.clone(), e))?;
            let path = entry.path();
            if path.is_file() && Self::is_process_script_path(&path) {
                paths.push(path);
            }
        }
        paths.sort_by(|a, b| {
            a.file_name()
                .unwrap_or_default()
                .cmp(b.file_name().unwrap_or_default())
        });

        paths
            .into_iter()
            .map(|p| Self::for_process_script_file(agent_dir, &p))
            .collect()
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
        assert_eq!(
            tool.input_schema_ref.as_deref(),
            Some("schemas/search_input.json")
        );
        assert_eq!(
            tool.output_schema_ref.as_deref(),
            Some("schemas/search_output.json")
        );
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
            ("mcp", "Mcp"),
        ] {
            let yaml = format!("name: t\nruntime: {val}\n");
            let tool: ToolDescriptor = serde_yaml::from_str(&yaml).unwrap();
            assert!(format!("{:?}", tool.runtime).contains(variant));
        }
    }

    #[test]
    fn parse_mcp_tool() {
        let yaml = "name: search\nruntime: mcp\nmcp_server: my-mcp\n";
        let tool: ToolDescriptor = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(tool.runtime, ToolRuntime::Mcp));
        assert_eq!(tool.mcp_server.as_deref(), Some("my-mcp"));
    }

    #[test]
    fn mcp_server_field_default_is_none() {
        let tool: ToolDescriptor = serde_yaml::from_str("name: t\n").unwrap();
        assert!(tool.mcp_server.is_none());
    }

    #[test]
    fn load_invalid_yaml_returns_parse_error() {
        let file = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        std::fs::write(file.path(), "name: [unclosed").unwrap();
        let err = ToolDescriptor::load(file.path()).unwrap_err();
        assert!(err.to_string().contains("Parse error"));
    }

    #[test]
    fn retry_policy_custom_max_attempts() {
        let yaml = "name: t\nretry_policy:\n  max_attempts: 5\n";
        let tool: ToolDescriptor = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(tool.retry_policy.max_attempts, 5);
    }

    #[test]
    fn capabilities_parsed() {
        let yaml = "name: t\ncapabilities:\n  - network\n  - filesystem\n";
        let tool: ToolDescriptor = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(tool.capabilities, vec!["network", "filesystem"]);
    }

    #[test]
    fn load_missing_file_returns_io_error() {
        let err = ToolDescriptor::load(Path::new("/nonexistent/tool.yaml")).unwrap_err();
        assert!(err.to_string().contains("IO error"));
    }

    #[test]
    fn discover_process_scripts_missing_tools_dir_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let tools = ToolDescriptor::discover_process_scripts(dir.path()).unwrap();
        assert!(tools.is_empty());
    }

    #[test]
    fn discover_process_scripts_ignores_non_scripts() {
        let dir = tempfile::tempdir().unwrap();
        let tools_sub = dir.path().join("tools");
        std::fs::create_dir(&tools_sub).unwrap();
        std::fs::write(tools_sub.join("README.md"), "docs").unwrap();
        std::fs::write(tools_sub.join("config.yaml"), "name: x\n").unwrap();

        let tools = ToolDescriptor::discover_process_scripts(dir.path()).unwrap();
        assert!(tools.is_empty());
    }

    #[test]
    fn discover_process_scripts_loads_py_files() {
        let dir = tempfile::tempdir().unwrap();
        let tools_sub = dir.path().join("tools");
        std::fs::create_dir(&tools_sub).unwrap();
        std::fs::write(tools_sub.join("alpha.py"), "# x").unwrap();

        let tools = ToolDescriptor::discover_process_scripts(dir.path()).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "alpha");
        assert_eq!(tools[0].module, "tools/alpha.py");
        assert!(matches!(tools[0].runtime, ToolRuntime::Process));
    }

    #[test]
    fn discover_process_scripts_sorts_by_filename() {
        let dir = tempfile::tempdir().unwrap();
        let tools_sub = dir.path().join("tools");
        std::fs::create_dir(&tools_sub).unwrap();
        std::fs::write(tools_sub.join("z.py"), "#").unwrap();
        std::fs::write(tools_sub.join("a.py"), "#").unwrap();

        let tools = ToolDescriptor::discover_process_scripts(dir.path()).unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "a");
        assert_eq!(tools[1].name, "z");
    }

    #[test]
    fn for_process_script_file_rejects_unsupported_extension() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("tools/x.rb");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "#").unwrap();
        let err = ToolDescriptor::for_process_script_file(dir.path(), &p).unwrap_err();
        assert!(err.to_string().contains("unsupported"));
    }
}
