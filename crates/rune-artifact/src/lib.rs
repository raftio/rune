//! Portable **rune-artifact** bundles: a versioned `manifest.json` plus packable agent files under
//! an `agent/` tree (same layout [`rune_spec::AgentPackage::load`] expects).
//!
//! Packed paths (relative to `agent/`) include `Runefile` and files under `tools/`, `skills/`, and
//! `schemas/` so process tools, local skills, and JSON schemas referenced from tool YAML are
//! shipped with the bundle.
//!
//! **Open Agent Initiative (OAI)** is the project’s name for this portable agent bundle format.
//! It is **not** an [Open Container Initiative](https://opencontainers.org/) (OCI) **container**
//! image; the payload is a deterministic bundle for agents, not a container runtime image.
//!
//! **Remote skills:** refs listed in `Runefile` but not present under `skills/` are **not**
//! embedded (only on-disk files under `skills/` are packed). After load, `missing_skills` is
//! populated the same as loading from a dev tree.
//!
//! Typical flow: [`materialize_agent_bundle`](crate::materialize_agent_bundle) → ship directory or
//! [`export_bundle_to_tar_gz_file`](crate::export_bundle_to_tar_gz_file) for a `.tar.gz` →
//! [`verify_dir`](crate::verify_dir) / [`verify`](crate::verify) → [`extract_to_temp`](crate::extract_to_temp) or
//! [`extract_and_load_package`](crate::extract_and_load_package) → run.

mod error;
mod manifest;
mod pack;
mod verify;

pub use error::ArtifactError;
pub use manifest::{FileEntry, Manifest, FORMAT_V1, INITIATIVE_OPEN_AGENT};
pub use pack::{
    export_bundle_to_tar_gz, export_bundle_to_tar_gz_file, materialize_agent_bundle, pack_agent_dir,
    pack_agent_dir_to_file, PackOptions, PackSummary,
};
pub use verify::{
    extract_and_load_package, extract_to_temp, read_manifest, read_manifest_dir, verify, verify_dir,
};

#[cfg(test)]
mod tests;
