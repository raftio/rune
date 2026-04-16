//! `manifest.json` schema for [`crate::FORMAT_V1`] artifacts.
//!
//! **Open Agent Initiative (OAI):** when present, [`INITIATIVE_OPEN_AGENT`] marks a pack aligned
//! with the project’s portable agent bundle naming (not [Open Container Initiative](https://opencontainers.org/) (OCI) container images).

use serde::{Deserialize, Serialize};

pub const FORMAT_V1: &str = "rune-artifact-v1";

/// Machine-readable label for OAI-aligned artifacts (`manifest.json` field `initiative`).
pub const INITIATIVE_OPEN_AGENT: &str = "open-agent";

/// One file entry under the `agent/` prefix (paths exclude the `agent/` prefix in JSON).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
}

/// Canonical manifest embedded as `agent/manifest.json` inside the tarball.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub format: String,
    pub agent_name: String,
    pub agent_version: String,
    pub created_at: String,
    /// Set to [`INITIATIVE_OPEN_AGENT`] for packs produced by current tooling; omitted in older artifacts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiative: Option<String>,
    /// Optional local label (e.g. git tag); not a remote registry reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub files: Vec<FileEntry>,
}

impl Manifest {
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec_pretty(self)
    }
}
