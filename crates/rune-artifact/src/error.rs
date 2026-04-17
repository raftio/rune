use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("spec error: {0}")]
    Spec(#[from] rune_spec::SpecError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("tar error: {0}")]
    Tar(String),

    #[error("missing manifest.json in artifact")]
    MissingManifest,

    #[error("invalid manifest format: {0}")]
    InvalidManifest(String),

    #[error("hash mismatch for {path}: expected {expected}, got {got}")]
    HashMismatch {
        path: String,
        expected: String,
        got: String,
    },

    #[error("Runefile not found under {0}")]
    MissingRunefile(PathBuf),

    #[error("no packable files under agent directory")]
    EmptyPackage,

    #[error("failed to read file {0}: {1}")]
    ReadFile(PathBuf, #[source] std::io::Error),

    #[error("failed to serialize manifest: {0}")]
    SerializeManifest(#[source] serde_json::Error),
}
