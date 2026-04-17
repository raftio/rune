use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::SpecError;
use crate::AgentSpec;

/// Agent definition: flattened [`AgentSpec`] including [`crate::ModelsSpec`] under `models:`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runefile {
    #[serde(flatten)]
    pub spec: AgentSpec,
}

impl Runefile {
    pub fn load(path: &Path) -> Result<Self, SpecError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| SpecError::Io(path.to_path_buf(), e))?;
        serde_yaml::from_str(&content)
            .map_err(|e| SpecError::Parse("Runefile".into(), e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runefile_yaml() -> &'static str {
        r"name: chat
version: 0.1.0
instructions: You are helpful.
default_model: default
models:
  model_mapping:
    default: claude-sonnet-4-6
"
    }

    #[test]
    fn parse_valid_runefile() {
        let rf: Runefile = serde_yaml::from_str(runefile_yaml()).unwrap();
        assert_eq!(rf.spec.name, "chat");
        assert_eq!(rf.spec.version, "0.1.0");
        assert_eq!(rf.spec.models.model_mapping["default"], "claude-sonnet-4-6");
    }

    #[test]
    fn load_from_file() {
        let file = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        std::fs::write(file.path(), runefile_yaml()).unwrap();
        let rf = Runefile::load(file.path()).unwrap();
        assert_eq!(rf.spec.name, "chat");
        assert_eq!(rf.spec.models.model_mapping["default"], "claude-sonnet-4-6");
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
    fn missing_instructions_returns_parse_error() {
        let file = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        std::fs::write(file.path(), "name: a\nversion: 0.1.0\n").unwrap();
        let err = Runefile::load(file.path()).unwrap_err();
        assert!(err.to_string().contains("Parse error"));
    }
}
