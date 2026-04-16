use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::{AgentSpec, ModelsSpec};
use crate::error::SpecError;

/// Single-file agent definition that merges spec, runtime, and models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runefile {
    #[serde(flatten)]
    pub spec: AgentSpec,
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
max_steps: 10
timeout_ms: 30000

models:
  providers:
    - openai
  model_mapping:
    default: gpt-4o-mini
  token_budget: 100000
"#
    }

    #[test]
    fn parse_valid_runefile() {
        let rf: Runefile = serde_yaml::from_str(runefile_yaml()).unwrap();
        assert_eq!(rf.spec.name, "chat");
        assert_eq!(rf.spec.version, "0.1.0");
        assert_eq!(rf.spec.max_steps, 10);
        assert_eq!(rf.models.providers, vec!["openai"]);
        assert_eq!(rf.models.model_mapping["default"], "gpt-4o-mini");
    }

    #[test]
    fn spec_fields_correctly_deserialized() {
        let rf: Runefile = serde_yaml::from_str(runefile_yaml()).unwrap();
        assert_eq!(rf.spec.timeout_ms, 30_000);
    }

    #[test]
    fn models_fields_correctly_deserialized() {
        let rf: Runefile = serde_yaml::from_str(runefile_yaml()).unwrap();
        assert_eq!(rf.models.token_budget, 100_000);
        assert!(matches!(rf.models.fallback_policy, crate::models::FallbackPolicy::NextProvider));
   }

    #[test]
    fn load_from_file_roundtrip() {
        let file = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        std::fs::write(file.path(), runefile_yaml()).unwrap();
        let rf = Runefile::load(file.path()).unwrap();
        assert_eq!(rf.spec.name, "chat");
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

    #[test]
    fn minimal_runtime_empty_map_uses_defaults() {
        let yaml = "name: a\nversion: 0.1.0\ninstructions: x\ndefault_model: d\nruntime: {}\nmodels: {}\n";
        let rf: Runefile = serde_yaml::from_str(yaml).unwrap();
        assert!(rf.models.providers.is_empty());
        assert_eq!(rf.models.token_budget, 100_000);
    }
}
