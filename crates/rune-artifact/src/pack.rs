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

/// Options for [`pack_agent_dir`], [`pack_agent_dir_to_file`], and [`materialize_agent_bundle`].
#[derive(Debug, Clone, Default)]
pub struct PackOptions {
    /// Optional local tag (e.g. release or git tag); stored in `manifest.json` for traceability only.
    pub tag: Option<String>,
}

/// Summary returned after a successful gzip-tar pack (SHA-256 of the entire `.tar.gz` stream).
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

#[derive(Debug)]
struct PreparedBundle {
    manifest_bytes: Vec<u8>,
    /// Sorted by path (same order as manifest `files` and tar member order after manifest).
    files: Vec<(String, Vec<u8>)>,
}

fn allowed_relative_path(rel: &str) -> bool {
    rel == "Runefile"
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

fn prepare_bundle_from_agent_dir(
    agent_dir: &Path,
    options: PackOptions,
) -> Result<PreparedBundle, ArtifactError> {
    if !agent_dir.join("Runefile").is_file() {
        return Err(ArtifactError::MissingRunefile(agent_dir.to_path_buf()));
    }
    let pkg = AgentPackage::load(agent_dir).map_err(|e| {
        if let rune_spec::SpecError::Io(_, ref io_err) = e {
            if io_err.kind() == std::io::ErrorKind::NotFound {
                return ArtifactError::MissingRunefile(agent_dir.to_path_buf());
            }
        }
        ArtifactError::Spec(e)
    })?;
    let paths = collect_packable_files(agent_dir)?;

    let mut file_entries: Vec<FileEntry> = Vec::with_capacity(paths.len());
    let mut files: Vec<(String, Vec<u8>)> = Vec::with_capacity(paths.len());
    for (full, rel) in paths {
        let bytes = std::fs::read(&full).map_err(|e| ArtifactError::ReadFile(full.clone(), e))?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let sha256 = hex::encode(hasher.finalize());
        file_entries.push(FileEntry {
            path: rel.clone(),
            sha256,
        });
        files.push((rel, bytes));
    }

    let manifest = Manifest {
        format: FORMAT_V1.to_string(),
        agent_name: pkg.spec.name.clone(),
        model: pkg.resolved_model()?,
        created_at: chrono::Utc::now().to_rfc3339(),
        initiative: Some(INITIATIVE_OPEN_AGENT.to_string()),
        tag: options.tag,
        files: file_entries,
    };

    let manifest_bytes = manifest
        .to_json_bytes()
        .map_err(ArtifactError::SerializeManifest)?;

    Ok(PreparedBundle {
        manifest_bytes,
        files,
    })
}

fn write_bundle_to_tar_gz(
    manifest_bytes: &[u8],
    files: &[(String, Vec<u8>)],
    out: impl Write,
) -> Result<PackSummary, ArtifactError> {
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
            manifest_bytes,
        )?;

        for (rel, bytes) in files {
            let path_in_tar = format!("{PREFIX}/{rel}");
            append_entry(&mut tar, &path_in_tar, bytes)?;
        }

        tar.finish()
            .map_err(|e| ArtifactError::Tar(e.to_string()))?;
        let gz = tar
            .into_inner()
            .map_err(|e| ArtifactError::Tar(format!("into_inner: {e}")))?;
        gz.finish()
            .map_err(|e| ArtifactError::Tar(format!("gz finish: {e}")))?;
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
    header
        .set_path(path)
        .map_err(|e| ArtifactError::Tar(e.to_string()))?;
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append(&header, bytes)
        .map_err(|e| ArtifactError::Tar(e.to_string()))?;
    Ok(())
}

/// Write an unpacked OAI bundle under `artifact_root`/`agent`/ (manifest + packable files).
///
/// Replaces any existing `artifact_root/agent/` directory. Parent directories are created as needed.
pub fn materialize_agent_bundle(
    agent_dir: &Path,
    artifact_root: &Path,
    options: PackOptions,
) -> Result<(), ArtifactError> {
    let b = prepare_bundle_from_agent_dir(agent_dir, options)?;
    let agent_out = artifact_root.join("agent");
    if agent_out.exists() {
        std::fs::remove_dir_all(&agent_out)?;
    }
    std::fs::create_dir_all(&agent_out)?;
    std::fs::write(agent_out.join("manifest.json"), &b.manifest_bytes)?;
    for (rel, bytes) in &b.files {
        let dest = agent_out.join(rel);
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&dest, bytes)?;
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
    let b = prepare_bundle_from_agent_dir(agent_dir, options)?;
    write_bundle_to_tar_gz(&b.manifest_bytes, &b.files, out)
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

/// Read a materialized bundle from `artifact_root` (must contain `agent/manifest.json` and listed
/// files) and write the same gzip-tar stream as [`pack_agent_dir`] would for equivalent content.
pub fn export_bundle_to_tar_gz(
    artifact_root: &Path,
    out: impl Write,
) -> Result<PackSummary, ArtifactError> {
    let manifest_path = artifact_root.join("agent").join("manifest.json");
    let manifest_bytes = std::fs::read(&manifest_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ArtifactError::MissingManifest
        } else {
            ArtifactError::Io(e)
        }
    })?;
    let m: Manifest = serde_json::from_slice(&manifest_bytes)?;
    if m.format != FORMAT_V1 {
        return Err(ArtifactError::InvalidManifest(format!(
            "expected format {FORMAT_V1}, got {}",
            m.format
        )));
    }

    let agent_root = artifact_root.join("agent");
    let mut pairs: Vec<(String, Vec<u8>)> = Vec::with_capacity(m.files.len());
    for f in &m.files {
        let full = agent_root.join(&f.path);
        let bytes = std::fs::read(&full).map_err(|e| ArtifactError::ReadFile(full.clone(), e))?;
        pairs.push((f.path.clone(), bytes));
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));

    write_bundle_to_tar_gz(&manifest_bytes, &pairs, out)
}

/// Convenience: export materialized bundle to a `.tar.gz` file path (overwrites).
pub fn export_bundle_to_tar_gz_file(
    artifact_root: &Path,
    out_path: &Path,
) -> Result<PackSummary, ArtifactError> {
    let f = File::create(out_path)?;
    export_bundle_to_tar_gz(artifact_root, f)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use flate2::read::GzDecoder;
    use tar::Archive;

    use super::*;

    fn write_minimal_agent(dir: &Path) {
        std::fs::write(
            dir.join("Runefile"),
            r"name: test-agent
version: 0.1.0
instructions: Hi.
default_model: default
models:
  model_mapping:
    default: claude-sonnet-4-6
",
        )
        .unwrap();
    }

    fn tar_entry_paths(buf: &[u8]) -> Vec<String> {
        let mut archive = Archive::new(GzDecoder::new(Cursor::new(buf)));
        archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    // --- allowed_relative_path ---

    #[test]
    fn allowed_runefile() {
        assert!(allowed_relative_path("Runefile"));
    }

    #[test]
    fn rejects_unknown_paths() {
        assert!(!allowed_relative_path("README.md"));
        assert!(!allowed_relative_path(".gitignore"));
        assert!(!allowed_relative_path("workflow.yaml"));
        assert!(!allowed_relative_path("tools/search.yaml"));
        assert!(!allowed_relative_path("skills/x.md"));
        assert!(!allowed_relative_path("src/main.rs"));
    }

    // --- collect_packable_files ---

    #[test]
    fn collect_missing_runefile_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = collect_packable_files(dir.path()).unwrap_err();
        assert!(matches!(err, ArtifactError::MissingRunefile(_)));
    }

    #[test]
    fn collect_excludes_non_allowed_files() {
        let dir = tempfile::tempdir().unwrap();
        write_minimal_agent(dir.path());
        std::fs::write(dir.path().join("README.md"), "# readme").unwrap();

        let files = collect_packable_files(dir.path()).unwrap();
        let rels: Vec<&str> = files.iter().map(|(_, r)| r.as_str()).collect();

        assert!(rels.contains(&"Runefile"));
        assert!(!rels.contains(&"README.md"));
    }

    // --- pack_agent_dir ---

    #[test]
    fn pack_missing_runefile_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = pack_agent_dir(dir.path(), std::io::sink(), PackOptions::default()).unwrap_err();
        assert!(matches!(err, ArtifactError::MissingRunefile(_)));
    }

    #[test]
    fn pack_invalid_runefile_returns_spec_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Runefile"), "{}").unwrap();
        let err = pack_agent_dir(dir.path(), std::io::sink(), PackOptions::default()).unwrap_err();
        assert!(matches!(err, ArtifactError::Spec(_)));
    }

    #[test]
    fn pack_excludes_non_allowed_files_from_tar() {
        let dir = tempfile::tempdir().unwrap();
        write_minimal_agent(dir.path());
        std::fs::write(dir.path().join("README.md"), "# readme").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();

        let mut buf = Vec::new();
        pack_agent_dir(dir.path(), &mut buf, PackOptions::default()).unwrap();

        let paths = tar_entry_paths(&buf);
        assert!(paths.iter().any(|p| p == "agent/Runefile"));
        assert!(!paths.iter().any(|p| p.contains("README.md")));
        assert!(!paths.iter().any(|p| p.contains(".gitignore")));
    }

    #[test]
    fn pack_summary_sha256_is_64_hex_chars() {
        let dir = tempfile::tempdir().unwrap();
        write_minimal_agent(dir.path());

        let summary = pack_agent_dir(dir.path(), std::io::sink(), PackOptions::default()).unwrap();
        assert_eq!(summary.artifact_sha256.len(), 64);
        assert!(summary
            .artifact_sha256
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn pack_manifest_entry_is_first_in_tar() {
        let dir = tempfile::tempdir().unwrap();
        write_minimal_agent(dir.path());

        let mut buf = Vec::new();
        pack_agent_dir(dir.path(), &mut buf, PackOptions::default()).unwrap();

        let paths = tar_entry_paths(&buf);
        assert_eq!(
            paths.first().map(String::as_str),
            Some("agent/manifest.json")
        );
    }

    // --- pack_agent_dir_to_file ---

    #[test]
    fn pack_to_file_writes_non_empty_artifact() {
        let src = tempfile::tempdir().unwrap();
        write_minimal_agent(src.path());
        let out = tempfile::NamedTempFile::new().unwrap();

        let summary =
            pack_agent_dir_to_file(src.path(), out.path(), PackOptions::default()).unwrap();
        assert_eq!(summary.artifact_sha256.len(), 64);
        assert!(out.path().metadata().unwrap().len() > 0);
    }

    #[test]
    fn pack_to_file_missing_runefile_returns_error() {
        let src = tempfile::tempdir().unwrap();
        let out = tempfile::NamedTempFile::new().unwrap();
        let err =
            pack_agent_dir_to_file(src.path(), out.path(), PackOptions::default()).unwrap_err();
        assert!(matches!(err, ArtifactError::MissingRunefile(_)));
    }

    #[test]
    fn read_manifest_reads_agent_name_without_hash_verify() {
        use std::io::Cursor;

        use crate::verify::read_manifest;

        let dir = tempfile::tempdir().unwrap();
        write_minimal_agent(dir.path());
        let mut buf = Vec::new();
        pack_agent_dir(dir.path(), &mut buf, PackOptions::default()).unwrap();
        let m = read_manifest(Cursor::new(&buf)).unwrap();
        assert_eq!(m.agent_name, "test-agent");
        assert!(!m.model.is_empty());
    }

    #[test]
    fn materialize_verify_dir_export_round_trip() {
        use crate::verify::{verify, verify_dir};

        let src = tempfile::tempdir().unwrap();
        write_minimal_agent(src.path());
        let bundle_root = tempfile::tempdir().unwrap();

        materialize_agent_bundle(src.path(), bundle_root.path(), PackOptions::default()).unwrap();
        let m_dir = verify_dir(bundle_root.path()).unwrap();
        assert_eq!(m_dir.agent_name, "test-agent");

        let mut exported = Vec::new();
        export_bundle_to_tar_gz(bundle_root.path(), &mut exported).unwrap();
        let m_tar = verify(Cursor::new(&exported)).unwrap();
        assert_eq!(m_tar.agent_name, "test-agent");
        assert_eq!(m_tar.files, m_dir.files);
    }
}
