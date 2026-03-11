use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpecError {
    #[error("IO error reading {0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),

    #[error("Parse error in {0}: {1}")]
    Parse(String, String),

    #[error("Validation error: {0}")]
    Validation(String),
}
