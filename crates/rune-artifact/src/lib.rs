//! Portable **rune-artifact** packs: gzip-compressed tarballs with a versioned `manifest.json`
//! and the same on-disk layout [`rune_spec::AgentPackage::load`] expects under an `agent/`
//! directory after extraction (`Runefile`, `tools/`, `skills/`, optional `workflow.yaml`).
//!
//! **Open Agent Initiative (OAI)** is the project’s name for this portable agent bundle format.
//! It is **not** an [Open Container Initiative](https://opencontainers.org/) (OCI) **container**
//! image; the payload is a deterministic archive for agents, not a container runtime image.
//!
//! **Remote skills:** refs listed in `Runefile` but not present under `skills/` are **not**
//! embedded in the tarball (MVP packs on-disk files only). After extract, `missing_skills` is
//! populated the same as loading from a dev tree.
//!
//! Typical flow: [`pack_agent_dir`](crate::pack_agent_dir) → ship `.tar.gz` →
//! [`verify`](crate::verify) → [`extract_to_temp`](crate::extract_to_temp) or
//! [`extract_and_load_package`](crate::extract_and_load_package) → run.

mod error;
mod manifest;
mod pack;
mod verify;

pub use error::ArtifactError;
pub use manifest::{FileEntry, Manifest, FORMAT_V1, INITIATIVE_OPEN_AGENT};
pub use pack::{pack_agent_dir, pack_agent_dir_to_file, PackOptions, PackSummary};
pub use verify::{extract_and_load_package, extract_to_temp, verify};

#[cfg(test)]
mod tests;
