use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use flate2::write::GzEncoder;
use flate2::Compression;
use rune_spec::AgentPackage;
use sha2::{Digest, Sha256};
use tar::Builder;

use crate::error::ArtifactError;
use crate::manifest::{FileEntry, Manifest, FORMAT_V1, INITIATIVE_OPEN_AGENT};

const PREFIX: &str = "agent";

/// Options for [`pack_agent_dir`] / [`pack_agent_dir_to_file`].
#[derive(Debug, Clone, Default)]
pub struct PackOptions {
    /// Optional local tag (e.g. release or git tag); stored in `manifest.json` for traceability only.
    pub tag: Option<String>,
}

/// Summary returned after a successful pack (SHA-256 of the entire `.tar.gz` stream).
#[derive(Debug, Clone)]
pub struct PackSummary {
    pub artifact_sha256: String,
}

struct HashWriter<W: Write> {
    inner: W,
    sha256: Sha256,
}

impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.sha256.update(buf);
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn allowed_relative_path(rel: &str) -> bool {
    rel == "Runefile"
        || rel == "workflow.yaml"
        || rel.starts_with("tools/")
        || rel.starts_with("skills/")
}

/// Collect (full_path, unix-style relative path) for every file that belongs in the artifact.
fn collect_packable_files(agent_dir: &Path) -> Result<Vec<(PathBuf, String)>, ArtifactError> {
    let runefile = agent_dir.join("Runefile");
    if !runefile.is_file() {
        return Err(ArtifactError::MissingRunefile(agent_dir.to_path_buf()));
    }

    let mut out: Vec<(PathBuf, String)> = Vec::new();
    walk_packable(agent_dir, agent_dir, &mut out)?;
    out.sort_by(|a, b| a.1.cmp(&b.1));
    if out.is_empty() {
        return Err(ArtifactError::EmptyPackage);
    }
    Ok(out)
}

fn walk_packable(
    base: &Path,
    dir: &Path,
    out: &mut Vec<(PathBuf, String)>,
) -> Result<(), ArtifactError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(base).expect("walk stays under base");
        let rel_str = rel.to_string_lossy().replace('\\', "/");

        if path.is_dir() {
            walk_packable(base, &path, out)?;
        } else if path.is_file() && allowed_relative_path(&rel_str) {
            out.push((path, rel_str));
        }
    }
    Ok(())
}

/// Validate the agent directory with [`AgentPackage::load`], then write a gzip-compressed tar
/// containing `agent/manifest.json` and all allowed files. Returns the hex SHA-256 of the
/// compressed bytes.
pub fn pack_agent_dir(
    agent_dir: &Path,
    out: impl Write,
    options: PackOptions,
) -> Result<PackSummary, ArtifactError> {
    let pkg = AgentPackage::load(agent_dir)?;
    let files = collect_packable_files(agent_dir)?;

    let mut file_entries: Vec<FileEntry> = Vec::with_capacity(files.len());
    for (full, rel) in &files {
        let bytes = std::fs::read(full)?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let sha256 = hex::encode(hasher.finalize());
        file_entries.push(FileEntry {
            path: rel.clone(),
            sha256,
        });
    }

    let manifest = Manifest {
        format: FORMAT_V1.to_string(),
        agent_name: pkg.spec.name.clone(),
        agent_version: pkg.spec.version.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        initiative: Some(INITIATIVE_OPEN_AGENT.to_string()),
        tag: options.tag,
        files: file_entries,
    };

    let manifest_bytes = manifest.to_json_bytes()?;

    let mut hw = HashWriter {
        inner: out,
        sha256: Sha256::new(),
    };
    {
        let gz = GzEncoder::new(&mut hw, Compression::default());
        let mut tar = Builder::new(gz);

        append_entry(
            &mut tar,
            &format!("{PREFIX}/manifest.json"),
            &manifest_bytes,
        )?;

        for (full, rel) in &files {
            let bytes = std::fs::read(full)?;
            let path_in_tar = format!("{PREFIX}/{rel}");
            append_entry(&mut tar, &path_in_tar, &bytes)?;
        }

        tar.finish().map_err(|e| ArtifactError::Tar(e.to_string()))?;
        let gz = tar
            .into_inner()
            .map_err(|e| ArtifactError::Tar(format!("into_inner: {e}")))?;
        gz.finish()?;
    }

    let artifact_sha256 = hex::encode(hw.sha256.finalize());
    Ok(PackSummary { artifact_sha256 })
}

fn append_entry<W: Write>(
    tar: &mut Builder<W>,
    path: &str,
    bytes: &[u8],
) -> Result<(), ArtifactError> {
    let mut header = tar::Header::new_gnu();
    header.set_path(path).map_err(|e| ArtifactError::Tar(e.to_string()))?;
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar
        .append(&header, bytes)
        .map_err(|e| ArtifactError::Tar(e.to_string()))?;
    Ok(())
}

/// Convenience: pack to a file path (overwrites).
pub fn pack_agent_dir_to_file(
    agent_dir: &Path,
    out_path: &Path,
    options: PackOptions,
) -> Result<PackSummary, ArtifactError> {
    let f = File::create(out_path)?;
    pack_agent_dir(agent_dir, f, options)
}
