use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::{AgentSpec, RuntimeSpec, ModelsSpec};
use crate::error::SpecError;

/// Single-file agent definition that merges spec, runtime, and models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runefile {
    #[serde(flatten)]
    pub spec: AgentSpec,
    pub runtime: RuntimeSpec,
    pub models: ModelsSpec,
}

impl Runefile {
    pub fn load(path: &Path) -> Result<Self, SpecError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| SpecError::Io(path.to_path_buf(), e))?;
        serde_yaml::from_str(&content)
            .map_err(|e| SpecError::Parse("Runefile".into(), e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runefile_yaml() -> &'static str {
        r#"
name: chat
version: 0.1.0
instructions: |
  You are a helpful assistant.
default_model: default
toolset: []
memory_profile: standard
max_steps: 10
timeout_ms: 30000

runtime:
  concurrency_limit: 10
  health_probe:
    path: /health
    interval_ms: 5000
  startup_timeout_ms: 10000
  request_timeout_ms: 30000
  streaming_enabled: true
  checkpoint_policy: on_finish
  resource_profile: small

models:
  providers:
    - openai
  model_mapping:
    default: gpt-4o-mini
  fallback_policy: next_provider
  token_budget: 100000
  safety_policy: standard
"#
    }

    #[test]
    fn parse_valid_runefile() {
        let rf: Runefile = serde_yaml::from_str(runefile_yaml()).unwrap();
        assert_eq!(rf.spec.name, "chat");
        assert_eq!(rf.spec.version, "0.1.0");
        assert_eq!(rf.spec.max_steps, 10);
        assert_eq!(rf.runtime.concurrency_limit, 10);
        assert_eq!(rf.runtime.health_probe.path, "/health");
        assert_eq!(rf.models.providers, vec!["openai"]);
        assert_eq!(rf.models.model_mapping["default"], "gpt-4o-mini");
    }

    #[test]
    fn spec_fields_correctly_deserialized() {
        let rf: Runefile = serde_yaml::from_str(runefile_yaml()).unwrap();
        assert_eq!(rf.spec.default_model, "default");
        assert!(rf.spec.toolset.is_empty());
        assert!(matches!(rf.spec.memory_profile, crate::agent::MemoryProfile::Standard));
        assert_eq!(rf.spec.timeout_ms, 30_000);
        assert_eq!(rf.spec.networks, vec!["bridge"]);
    }

    #[test]
    fn runtime_fields_correctly_deserialized() {
        let rf: Runefile = serde_yaml::from_str(runefile_yaml()).unwrap();
        assert!(rf.runtime.streaming_enabled);
        assert!(matches!(rf.runtime.checkpoint_policy, crate::runtime::CheckpointPolicy::OnFinish));
        assert!(matches!(rf.runtime.resource_profile, crate::runtime::ResourceProfile::Small));
    }

    #[test]
    fn models_fields_correctly_deserialized() {
        let rf: Runefile = serde_yaml::from_str(runefile_yaml()).unwrap();
        assert_eq!(rf.models.token_budget, 100_000);
        assert!(matches!(rf.models.fallback_policy, crate::models::FallbackPolicy::NextProvider));
        assert!(matches!(rf.models.safety_policy, crate::models::SafetyPolicy::Standard));
    }

    #[test]
    fn load_from_file_roundtrip() {
        let file = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        std::fs::write(file.path(), runefile_yaml()).unwrap();
        let rf = Runefile::load(file.path()).unwrap();
        assert_eq!(rf.spec.name, "chat");
        assert_eq!(rf.runtime.concurrency_limit, 10);
        assert_eq!(rf.models.providers, vec!["openai"]);
    }

    #[test]
    fn load_missing_file_returns_io_error() {
        let err = Runefile::load(Path::new("/nonexistent/Runefile")).unwrap_err();
        assert!(err.to_string().contains("IO error"));
    }

    #[test]
    fn invalid_yaml_returns_parse_error() {
        let file = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        std::fs::write(file.path(), "not: valid: yaml: [").unwrap();
        let err = Runefile::load(file.path()).unwrap_err();
        assert!(err.to_string().contains("Parse error"));
    }
}
