use std::path::{Path, PathBuf};

use rune_spec::{AgentPackage, ModelsSpec, ToolDescriptor};

use crate::error::RuntimeError;

#[cfg(test)]
mod tests {
    use super::*;

    // --- stub ---

    #[test]
    fn stub_sets_correct_agent_name() {
        let plan = ExecutionPlan::stub("my-agent");
        assert_eq!(plan.agent_name, "my-agent");
    }

    #[test]
    fn stub_has_sensible_defaults() {
        let plan = ExecutionPlan::stub("test");
        assert_eq!(plan.max_steps, 10);
        assert_eq!(plan.timeout_ms, 30_000);
        assert!(!plan.instructions.is_empty());
        assert!(!plan.default_model.is_empty());
    }

    #[test]
    fn stub_has_bridge_network() {
        let plan = ExecutionPlan::stub("test");
        assert_eq!(plan.networks, vec!["bridge"]);
    }

    #[test]
    fn stub_has_empty_tools_and_toolset() {
        let plan = ExecutionPlan::stub("test");
        assert!(plan.tools.is_empty());
        assert!(plan.toolset.is_empty());
    }

    #[test]
    fn stub_models_spec_is_default() {
        let plan = ExecutionPlan::stub("test");
        assert!(plan.models.providers.is_empty());
    }

    // --- from_dir ---

    fn write_minimal_agent(dir: &std::path::Path) {
        std::fs::write(
            dir.join("Runefile"),
            "name: test-agent\nversion: 0.1.0\ninstructions: You are a test agent.\ndefault_model: default\nruntime: {}\nmodels: {}\n",
        )
        .unwrap();
    }

    #[test]
    fn from_dir_loads_minimal_package() {
        let dir = tempfile::tempdir().unwrap();
        write_minimal_agent(dir.path());

        let plan = ExecutionPlan::from_dir(dir.path()).unwrap();
        assert_eq!(plan.agent_name, "test-agent");
        assert_eq!(plan.max_steps, 20); // spec.yaml default
        assert_eq!(plan.timeout_ms, 30_000);
        assert_eq!(plan.networks, vec!["bridge"]);
    }

    #[test]
    fn from_dir_missing_dir_returns_spec_error() {
        let result = ExecutionPlan::from_dir(std::path::Path::new("/nonexistent/agent"));
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("Spec error") || msg.contains("IO error"));
    }

    #[test]
    fn from_dir_toolset_merges_spec_and_tools_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Runefile"),
            "name: agent\nversion: 0.1.0\ninstructions: x\ndefault_model: d\ntoolset:\n  - rune@shell\nruntime: {}\nmodels: {}\n",
        )
        .unwrap();

        let tools_dir = dir.path().join("tools");
        std::fs::create_dir(&tools_dir).unwrap();
        std::fs::write(
            tools_dir.join("search.yaml"),
            "name: my_search\n",
        )
        .unwrap();

        let plan = ExecutionPlan::from_dir(dir.path()).unwrap();
        assert!(plan.toolset.contains(&"rune@shell".to_string()));
        assert!(plan.toolset.contains(&"my_search".to_string()));
    }

    #[test]
    fn from_dir_with_runefile_uses_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Runefile"),
            "name: runefile-agent\nversion: 0.2.0\ninstructions: From Runefile.\ndefault_model: default\nruntime:\n  concurrency_limit: 5\nmodels: {}\n",
        )
        .unwrap();

        let plan = ExecutionPlan::from_dir(dir.path()).unwrap();
        assert_eq!(plan.agent_name, "runefile-agent");
    }
}

/// In-memory execution plan built from an agent package.
pub struct ExecutionPlan {
    pub agent_name: String,
    pub instructions: String,
    pub default_model: String,
    pub max_steps: u32,
    pub timeout_ms: u64,
    pub tools: Vec<ToolDescriptor>,
    pub agent_dir: PathBuf,
    pub models: ModelsSpec,
    pub toolset: Vec<String>,
    /// rune-network memberships for this agent (default: ["bridge"]).
    pub networks: Vec<String>,
}

impl ExecutionPlan {
    /// Load from a local agent directory containing a `Runefile`.
    pub fn from_dir(agent_dir: &Path) -> Result<Self, RuntimeError> {
        let pkg = AgentPackage::load(agent_dir)
            .map_err(|e| RuntimeError::Spec(e.to_string()))?;
        let mut toolset: Vec<String> = pkg.spec.toolset.clone();
        for t in &pkg.tools {
            if !toolset.contains(&t.name) {
                toolset.push(t.name.clone());
            }
        }
        Ok(Self {
            agent_name: pkg.spec.name,
            instructions: pkg.spec.instructions,
            default_model: pkg.spec.default_model,
            max_steps: pkg.spec.max_steps,
            timeout_ms: pkg.spec.timeout_ms,
            tools: pkg.tools,
            agent_dir: agent_dir.to_path_buf(),
            models: pkg.models,
            toolset,
            networks: pkg.spec.networks,
        })
    }

    /// Minimal stub plan — used when the agent package is not locally available
    /// (e.g. during Phase 3 before OCI fetch is implemented).
    pub fn stub(agent_name: impl Into<String>) -> Self {
        Self {
            agent_name: agent_name.into(),
            instructions: "You are a helpful assistant.".into(),
            default_model: "claude-sonnet-4-6".into(),
            max_steps: 10,
            timeout_ms: 30_000,
            tools: vec![],
            agent_dir: PathBuf::new(),
            models: ModelsSpec::default(),
            toolset: vec![],
            networks: vec!["bridge".into()],
        }
    }
}
