pub mod agent;
pub mod error;
pub mod models;
pub mod runefile;
pub mod tool;
pub mod workflow;

pub use agent::AgentSpec;
pub use models::{FallbackPolicy, ModelsSpec};
pub use tool::{RetryPolicy, ToolDescriptor, ToolRuntime};
pub use workflow::{topological_sort, WorkflowSpec, WorkflowStep};

/// Type alias for [`ModelsSpec`] — the `models:` block in a Runefile (providers, `model_mapping`, etc.).
pub type ModelSpec = ModelsSpec;
pub use error::SpecError;
pub use runefile::Runefile;
use std::path::Path;

/// Full agent package loaded from a `Runefile`.
///
/// Model configuration lives in [`AgentSpec::models`] ([`ModelsSpec`]). The concrete model id for
/// the configured [`AgentSpec::default_model`] alias is returned by [`AgentPackage::resolved_model`].
#[derive(Debug)]
pub struct AgentPackage {
    pub spec: AgentSpec,
}

fn resolve_default_model(spec: &AgentSpec) -> Result<String, SpecError> {
    spec.models
        .model_mapping
        .get(spec.default_model.as_str())
        .cloned()
        .ok_or_else(|| {
            SpecError::Validation(format!(
                "models.model_mapping has no entry for default_model alias {:?}",
                spec.default_model
            ))
        })
}

impl AgentPackage {
    pub fn load(agent_dir: &Path) -> Result<Self, SpecError> {
        let rf = Runefile::load(&agent_dir.join("Runefile"))?;
        resolve_default_model(&rf.spec)?;
        Ok(Self { spec: rf.spec })
    }

    /// Load from an explicit Runefile path (any filename); validates default model mapping.
    pub fn load_runefile(path: &Path) -> Result<Self, SpecError> {
        let rf = Runefile::load(path)?;
        resolve_default_model(&rf.spec)?;
        Ok(Self { spec: rf.spec })
    }

    /// Concrete model id for [`AgentSpec::default_model`] via [`ModelsSpec::model_mapping`].
    pub fn resolved_model(&self) -> Result<String, SpecError> {
        resolve_default_model(&self.spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn load_package_requires_model_mapping_for_default_alias() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("Runefile"),
            r"name: a
version: 0.1.0
instructions: x
default_model: default
models:
  model_mapping: {}
",
        )
        .unwrap();
        let err = AgentPackage::load(dir.path()).unwrap_err();
        assert!(matches!(err, SpecError::Validation(_)));
    }
}
