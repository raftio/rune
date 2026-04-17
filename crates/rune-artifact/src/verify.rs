use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use tar::Archive;

use crate::error::ArtifactError;
use crate::manifest::{Manifest, FORMAT_V1};

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Read a `.tar.gz` artifact, parse `agent/manifest.json`, and verify every listed file hash.
pub fn verify(mut artifact: impl Read) -> Result<Manifest, ArtifactError> {
    let mut data = Vec::new();
    artifact.read_to_end(&mut data)?;
    let decoder = GzDecoder::new(Cursor::new(data));
    let mut archive = Archive::new(decoder);

    let mut manifest: Option<Manifest> = None;
    let mut contents: HashMap<String, Vec<u8>> = HashMap::new();

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().into_owned();
        if path == "agent/manifest.json" {
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            manifest = Some(serde_json::from_str(&s)?);
            continue;
        }
        if let Some(rel) = path.strip_prefix("agent/") {
            if rel == "manifest.json" {
                continue;
            }
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            contents.insert(rel.to_string(), buf);
        }
    }

    let m = manifest.ok_or(ArtifactError::MissingManifest)?;
    if m.format != FORMAT_V1 {
        return Err(ArtifactError::InvalidManifest(format!(
            "expected format {FORMAT_V1}, got {}",
            m.format
        )));
    }

    for f in &m.files {
        let bytes = contents
            .get(&f.path)
            .ok_or_else(|| ArtifactError::InvalidManifest(format!("missing path {}", f.path)))?;
        let got = sha256_hex(bytes);
        if got != f.sha256 {
            return Err(ArtifactError::HashMismatch {
                path: f.path.clone(),
                expected: f.sha256.clone(),
                got,
            });
        }
    }

    Ok(m)
}

/// Read `agent/manifest.json` from a materialized bundle directory and verify every listed file hash.
pub fn verify_dir(artifact_root: &Path) -> Result<Manifest, ArtifactError> {
    let manifest_path = artifact_root.join("agent").join("manifest.json");
    let bytes = std::fs::read(&manifest_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ArtifactError::MissingManifest
        } else {
            ArtifactError::Io(e)
        }
    })?;
    let m: Manifest = serde_json::from_slice(&bytes)?;
    if m.format != FORMAT_V1 {
        return Err(ArtifactError::InvalidManifest(format!(
            "expected format {FORMAT_V1}, got {}",
            m.format
        )));
    }

    let agent = artifact_root.join("agent");
    for f in &m.files {
        let full = agent.join(&f.path);
        let file_bytes = std::fs::read(&full).map_err(|e| ArtifactError::ReadFile(full.clone(), e))?;
        let got = sha256_hex(&file_bytes);
        if got != f.sha256 {
            return Err(ArtifactError::HashMismatch {
                path: f.path.clone(),
                expected: f.sha256.clone(),
                got,
            });
        }
    }

    Ok(m)
}

/// Parse `agent/manifest.json` from a materialized bundle directory without verifying file hashes.
pub fn read_manifest_dir(artifact_root: &Path) -> Result<Manifest, ArtifactError> {
    let manifest_path = artifact_root.join("agent").join("manifest.json");
    let bytes = std::fs::read(&manifest_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ArtifactError::MissingManifest
        } else {
            ArtifactError::Io(e)
        }
    })?;
    let m: Manifest = serde_json::from_slice(&bytes)?;
    if m.format != FORMAT_V1 {
        return Err(ArtifactError::InvalidManifest(format!(
            "expected format {FORMAT_V1}, got {}",
            m.format
        )));
    }
    Ok(m)
}

/// Parse `agent/manifest.json` from a `.tar.gz` without verifying file hashes (for listing).
///
/// Other archive members are skipped without loading their payloads into memory.
pub fn read_manifest(mut artifact: impl Read) -> Result<Manifest, ArtifactError> {
    let mut data = Vec::new();
    artifact.read_to_end(&mut data)?;
    let decoder = GzDecoder::new(Cursor::new(data));
    let mut archive = Archive::new(decoder);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().into_owned();
        if path == "agent/manifest.json" {
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            let m: Manifest = serde_json::from_str(&s)?;
            if m.format != FORMAT_V1 {
                return Err(ArtifactError::InvalidManifest(format!(
                    "expected format {FORMAT_V1}, got {}",
                    m.format
                )));
            }
            return Ok(m);
        }
        std::io::copy(&mut entry, &mut std::io::sink())?;
    }

    Err(ArtifactError::MissingManifest)
}

/// Decompress and unpack the artifact into a fresh temp directory; returns the path to the
/// `agent/` root (contains `Runefile`, etc.) suitable for [`rune_spec::AgentPackage::load`].
pub fn extract_to_temp(
    mut artifact: impl Read,
) -> Result<(tempfile::TempDir, std::path::PathBuf), ArtifactError> {
    let mut data = Vec::new();
    artifact.read_to_end(&mut data)?;
    let decoder = GzDecoder::new(Cursor::new(data));
    let mut archive = Archive::new(decoder);

    let tmp = tempfile::tempdir()?;
    let dest = tmp.path();
    archive.unpack(dest)?;

    let agent_root = dest.join("agent");
    if !agent_root.join("Runefile").is_file() {
        return Err(ArtifactError::MissingRunefile(agent_root));
    }

    Ok((tmp, agent_root))
}

/// Extract then load the package (keeps temp dir alive in returned tuple).
pub fn extract_and_load_package(
    artifact: impl Read,
) -> Result<(tempfile::TempDir, rune_spec::AgentPackage), ArtifactError> {
    let (tmp, dir) = extract_to_temp(artifact)?;
    let pkg = rune_spec::AgentPackage::load(&dir)?;
    Ok((tmp, pkg))
}
