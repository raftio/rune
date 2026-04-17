use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use rune_artifact::PackOptions;
use rune_spec::AgentPackage;

use crate::cli::{ArtifactBuildArgs, ArtifactExportArgs, ArtifactInspectArgs, ArtifactRemoveArgs};

fn sanitize_filename_component(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "agent".into()
    } else {
        s
    }
}

fn artifacts_root() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("could not resolve home directory")?
        .join(".rune")
        .join("artifacts"))
}

/// ~/.rune/artifacts/{name}-{tag}/ (sanitized components)
fn artifact_bundle_dir(agent_name: &str, tag: &str) -> Result<PathBuf> {
    let base = artifacts_root()?;
    let name = sanitize_filename_component(agent_name);
    let tag = sanitize_filename_component(tag);
    Ok(base.join(format!("{name}-{tag}")))
}

/// ~/.rune/artifacts/{name}-{tag}.tar.gz (sanitized components)
fn artifact_tar_path(agent_name: &str, tag: &str) -> Result<PathBuf> {
    let base = artifacts_root()?;
    let name = sanitize_filename_component(agent_name);
    let tag = sanitize_filename_component(tag);
    Ok(base.join(format!("{name}-{tag}.tar.gz")))
}

fn prepare_artifacts_parent() -> Result<PathBuf> {
    let base = artifacts_root()?;
    std::fs::create_dir_all(&base).with_context(|| format!("create {}", base.display()))?;
    Ok(base)
}

/// Ensures ~/.rune/artifacts exists and returns the materialized bundle root directory path.
fn prepare_bundle_output_dir(agent_name: &str, tag: &str) -> Result<PathBuf> {
    prepare_artifacts_parent()?;
    Ok(artifact_bundle_dir(agent_name, tag)?)
}

fn bundle_manifest_path(bundle_root: &Path) -> PathBuf {
    bundle_root.join("agent").join("manifest.json")
}

fn is_materialized_bundle(bundle_root: &Path) -> bool {
    bundle_manifest_path(bundle_root).is_file()
}

fn format_mtime(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| secs.to_string())
}

struct LsRow {
    agent: String,
    tag: String,
    model: String,
    size_kb: String,
    mtime: String,
}

/// Total size of files under `path` (recursive). Returns error if `path` is not a directory.
fn dir_size_bytes(path: &Path) -> Result<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(path).with_context(|| format!("read_dir {}", path.display()))? {
        let entry = entry.with_context(|| format!("read entry in {}", path.display()))?;
        let meta = entry
            .metadata()
            .with_context(|| format!("stat {}", entry.path().display()))?;
        if meta.is_file() {
            total += meta.len();
        } else if meta.is_dir() {
            total += dir_size_bytes(&entry.path())?;
        }
    }
    Ok(total)
}

/// File size in kibibytes (1024 B), one decimal place.
fn format_size_kb(bytes: u64) -> String {
    let kb = bytes as f64 / 1024.0;
    format!("{kb:.1}")
}

fn print_ls_table(rows: &[LsRow]) {
    const H_AGENT: &str = "AGENT";
    const H_TAG: &str = "TAG";
    const H_MODEL: &str = "MODEL";
    const H_SIZE: &str = "KB";
    const H_MTIME: &str = "MODIFIED (UTC)";

    let wa = H_AGENT
        .len()
        .max(
            rows.iter()
                .map(|r| r.agent.chars().count())
                .max()
                .unwrap_or(0),
        )
        .max(5);
    let wt = H_TAG
        .len()
        .max(
            rows.iter()
                .map(|r| r.tag.chars().count())
                .max()
                .unwrap_or(0),
        )
        .max(3);
    let wm = H_MODEL
        .len()
        .max(
            rows.iter()
                .map(|r| r.model.chars().count())
                .max()
                .unwrap_or(0),
        )
        .max(8)
        .min(48);
    let wb = H_SIZE.len().max(
        rows.iter()
            .map(|r| r.size_kb.chars().count())
            .max()
            .unwrap_or(0),
    );
    let wmt = H_MTIME.len().max(
        rows.iter()
            .map(|r| r.mtime.chars().count())
            .max()
            .unwrap_or(0),
    );

    const GAP: &str = "  ";

    fn trunc(s: &str, max_chars: usize) -> String {
        let count = s.chars().count();
        if count <= max_chars {
            return s.to_string();
        }
        let take = max_chars.saturating_sub(1);
        let prefix: String = s.chars().take(take).collect();
        format!("{prefix}…")
    }

    println!(
        "{:<wa$}{GAP}{:<wt$}{GAP}{:<wm$}{GAP}{:>wb$}{GAP}{:<wmt$}",
        H_AGENT,
        H_TAG,
        H_MODEL,
        H_SIZE,
        H_MTIME,
        wa = wa,
        wt = wt,
        wm = wm,
        wb = wb,
        wmt = wmt,
        GAP = GAP
    );
    println!(
        "{}{}{}{}{}{}{}{}{}",
        "-".repeat(wa),
        GAP,
        "-".repeat(wt),
        GAP,
        "-".repeat(wm),
        GAP,
        "-".repeat(wb),
        GAP,
        "-".repeat(wmt),
    );
    for r in rows {
        println!(
            "{:<wa$}{GAP}{:<wt$}{GAP}{:<wm$}{GAP}{:>wb$}{GAP}{:<wmt$}",
            r.agent,
            r.tag,
            trunc(&r.model, wm),
            r.size_kb,
            r.mtime,
            wa = wa,
            wt = wt,
            wm = wm,
            wb = wb,
            wmt = wmt,
            GAP = GAP
        );
    }
}

pub fn ls() -> Result<()> {
    let dir = artifacts_root()?;
    if !dir.exists() {
        println!("(no artifacts directory; {})", dir.display());
        return Ok(());
    }
    if !dir.is_dir() {
        anyhow::bail!("{} is not a directory", dir.display());
    }

    let mut rows: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
    for entry in std::fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry.with_context(|| format!("read entry in {}", dir.display()))?;
        let path = entry.path();
        let meta = entry
            .metadata()
            .with_context(|| format!("stat {}", path.display()))?;

        if meta.is_dir() {
            if !is_materialized_bundle(&path) {
                continue;
            }
            let size = dir_size_bytes(&path).unwrap_or(0);
            let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
            rows.push((path, size, mtime));
            continue;
        }

        if !meta.is_file() {
            continue;
        }
        let fname = entry.file_name();
        let Some(name) = fname.to_str() else {
            continue;
        };
        if !name.ends_with(".tar.gz") {
            continue;
        }
        let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
        rows.push((path, meta.len(), mtime));
    }

    rows.sort_by(|a, b| a.0.cmp(&b.0));

    if rows.is_empty() {
        println!("(no bundles or .tar.gz artifacts in {})", dir.display());
        return Ok(());
    }

    let mut out: Vec<LsRow> = Vec::new();
    for (path, size, mtime) in rows {
        let (agent, tag, model) = if path.is_dir() {
            match rune_artifact::read_manifest_dir(&path) {
                Ok(m) => (
                    m.agent_name,
                    m.tag.unwrap_or_else(|| "-".to_string()),
                    m.model,
                ),
                Err(_) => ("?".into(), "?".into(), "?".into()),
            }
        } else {
            match File::open(&path) {
                Ok(f) => match rune_artifact::read_manifest(f) {
                    Ok(m) => (
                        m.agent_name,
                        m.tag.unwrap_or_else(|| "-".to_string()),
                        m.model,
                    ),
                    Err(_) => ("?".into(), "?".into(), "?".into()),
                },
                Err(_) => ("?".into(), "?".into(), "?".into()),
            }
        };
        out.push(LsRow {
            agent,
            tag,
            model,
            size_kb: format_size_kb(size),
            mtime: format_mtime(mtime),
        });
    }
    print_ls_table(&out);
    Ok(())
}

#[derive(Debug)]
enum StoredBundle {
    Dir(PathBuf),
    Tar(PathBuf),
}

/// Load an agent package from `~/.rune/artifacts` by manifest `agent_name`.
/// Prefers tag `latest`, then newest mtime.
pub fn load_by_stored_agent_name(
    name: &str,
) -> Result<(Option<tempfile::TempDir>, AgentPackage)> {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        anyhow::bail!(
            "invalid artifact name {:?}: use a plain agent name, a filesystem path, or git://...",
            name
        );
    }

    let dir = artifacts_root()?;
    if !dir.is_dir() {
        anyhow::bail!(
            "no stored artifact {:?}: artifacts directory {} does not exist; use a path, `rune artifact build`, or git://...",
            name,
            dir.display()
        );
    }

    let mut candidates: Vec<(StoredBundle, Option<String>, std::time::SystemTime)> = Vec::new();

    for entry in std::fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry.with_context(|| format!("read entry in {}", dir.display()))?;
        let path = entry.path();
        let meta = entry
            .metadata()
            .with_context(|| format!("stat {}", path.display()))?;

        if meta.is_dir() {
            if !is_materialized_bundle(&path) {
                continue;
            }
            let m = match rune_artifact::read_manifest_dir(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if m.agent_name != name {
                continue;
            }
            let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
            candidates.push((StoredBundle::Dir(path), m.tag, mtime));
            continue;
        }

        if !meta.is_file() {
            continue;
        }
        let fname = entry.file_name();
        let Some(fname_str) = fname.to_str() else {
            continue;
        };
        if !fname_str.ends_with(".tar.gz") {
            continue;
        }
        let m = match File::open(&path) {
            Ok(f) => match rune_artifact::read_manifest(f) {
                Ok(m) => m,
                Err(_) => continue,
            },
            Err(_) => continue,
        };
        if m.agent_name != name {
            continue;
        }
        let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
        candidates.push((StoredBundle::Tar(path), m.tag, mtime));
    }

    if candidates.is_empty() {
        anyhow::bail!(
            "no stored artifact with agent name {:?} under {}; use `rune artifact ls`, a local path, or git://...",
            name,
            dir.display()
        );
    }

    candidates.sort_by(|a, b| {
        let a_latest = a.1.as_deref() == Some("latest");
        let b_latest = b.1.as_deref() == Some("latest");
        b_latest
            .cmp(&a_latest)
            .then_with(|| {
                let ta = a.2.duration_since(UNIX_EPOCH).unwrap_or_default();
                let tb = b.2.duration_since(UNIX_EPOCH).unwrap_or_default();
                tb.cmp(&ta)
            })
    });

    match &candidates[0].0 {
        StoredBundle::Dir(root) => {
            let pkg = AgentPackage::load(&root.join("agent"))
                .with_context(|| format!("load agent from {}", root.display()))?;
            Ok((None, pkg))
        }
        StoredBundle::Tar(tar_path) => {
            let f = File::open(tar_path).with_context(|| format!("open {}", tar_path.display()))?;
            let (tmp, pkg) = rune_artifact::extract_and_load_package(f)
                .with_context(|| format!("extract artifact {}", tar_path.display()))?;
            Ok((Some(tmp), pkg))
        }
    }
}

pub fn build(args: ArtifactBuildArgs) -> Result<()> {
    let agent_dir = args.agent_dir;
    let tag = args.tag.unwrap_or_else(|| "latest".to_string());

    let pkg = AgentPackage::load(&agent_dir)
        .with_context(|| format!("load agent from {}", agent_dir.display()))?;

    let output = prepare_bundle_output_dir(&pkg.spec.name, &tag)?;

    rune_artifact::materialize_agent_bundle(
        &agent_dir,
        &output,
        PackOptions {
            tag: Some(tag.clone()),
        },
    )
    .with_context(|| format!("materialize agent bundle from {}", agent_dir.display()))?;

    println!("agent_name={}", pkg.spec.name);
    println!("version={}", pkg.spec.version);
    println!(
        "model={}",
        pkg.resolved_model()
            .with_context(|| "resolve default_model in models.model_mapping")?
    );
    println!("tag={tag}");
    println!("{}", output.display());
    Ok(())
}

pub fn export_cmd(args: ArtifactExportArgs) -> Result<()> {
    let bundle_root = artifact_bundle_dir(&args.name, &args.tag)?;
    if !is_materialized_bundle(&bundle_root) {
        anyhow::bail!(
            "bundle not found at {} (expected agent/manifest.json); run `rune artifact build` first",
            bundle_root.display()
        );
    }

    let out_path = if let Some(p) = args.output {
        p
    } else {
        artifact_tar_path(&args.name, &args.tag)?
    };
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }

    let summary = rune_artifact::export_bundle_to_tar_gz_file(&bundle_root, &out_path)
        .with_context(|| format!("export bundle {}", bundle_root.display()))?;

    println!("artifact_sha256={}", summary.artifact_sha256);
    println!("{}", out_path.display());
    Ok(())
}

fn print_inspect_manifest(m: &rune_artifact::Manifest) {
    println!("format: {}", m.format);
    if let Some(ref i) = m.initiative {
        println!("initiative: {i}");
    }
    if let Some(ref t) = m.tag {
        println!("tag: {t}");
    }
    println!("agent_name: {}", m.agent_name);
    println!("model: {}", m.model);
    println!("created_at: {}", m.created_at);
    println!("files: {}", m.files.len());
}

pub fn inspect_cmd(args: ArtifactInspectArgs) -> Result<()> {
    let bundle_root = artifact_bundle_dir(&args.name, &args.tag)?;
    if is_materialized_bundle(&bundle_root) {
        let m = rune_artifact::verify_dir(&bundle_root)
            .with_context(|| format!("verify bundle directory {}", bundle_root.display()))?;
        print_inspect_manifest(&m);
        return Ok(());
    }

    let tar_path = artifact_tar_path(&args.name, &args.tag)?;
    let f = File::open(&tar_path).with_context(|| {
        format!(
            "open artifact: no bundle at {} and no archive at {}",
            bundle_root.display(),
            tar_path.display()
        )
    })?;
    let m = rune_artifact::verify(f)
        .with_context(|| format!("verify artifact archive {}", tar_path.display()))?;
    print_inspect_manifest(&m);
    Ok(())
}

fn deployment_still_active(d: &serde_json::Value) -> bool {
    let desired = d["desired_replicas"].as_i64().unwrap_or(0);
    let status = d["status"].as_str().unwrap_or("");
    desired > 0 || status != "stopped"
}

/// Block removal when any non-stopped deployment references an agent-version whose
/// `agent_name` + `spec_sha256` match this bundle (same registration as `rune run`).
async fn ensure_no_deployments_using_bundle(
    base: &str,
    pkg: &AgentPackage,
    fingerprint: &str,
) -> Result<()> {
    let http = reqwest::Client::new();
    let (deployments, versions) =
        crate::commands::agent::fetch_control_plane_state(&http, base).await?;

    let matching_version_ids: HashSet<String> = versions
        .iter()
        .filter(|v| {
            v["agent_name"].as_str() == Some(pkg.spec.name.as_str())
                && v["spec_sha256"].as_str() == Some(fingerprint)
        })
        .filter_map(|v| v["id"].as_str().map(std::string::ToString::to_string))
        .collect();

    if matching_version_ids.is_empty() {
        return Ok(());
    }

    let mut lines: Vec<String> = Vec::new();
    for d in deployments {
        let Some(vid) = d["agent_version_id"].as_str() else {
            continue;
        };
        if !matching_version_ids.contains(vid) {
            continue;
        }
        if !deployment_still_active(&d) {
            continue;
        }
        let id = d["id"].as_str().unwrap_or("?");
        let ns = d["namespace"].as_str().unwrap_or("?");
        let alias = d["rollout_alias"].as_str().unwrap_or("?");
        let status = d["status"].as_str().unwrap_or("?");
        let desired = d["desired_replicas"].as_i64().unwrap_or(0);
        lines.push(format!(
            "{id}  namespace={ns}  alias={alias}  status={status}  desired_replicas={desired}"
        ));
    }

    if lines.is_empty() {
        return Ok(());
    }

    anyhow::bail!(
        "cannot remove: deployment(s) still use this bundle (same spec as `rune run`):\n  {}\n\
         Stop or remove those deployments first, or pass --force.",
        lines.join("\n  ")
    );
}

pub async fn remove_cmd(args: ArtifactRemoveArgs) -> Result<()> {
    let bundle_root = artifact_bundle_dir(&args.name, &args.tag)?;

    if !bundle_root.exists() {
        anyhow::bail!(
            "no materialized bundle at {}; run `rune artifact build` first",
            bundle_root.display()
        );
    }
    if !bundle_root.is_dir() {
        anyhow::bail!("{} exists but is not a directory", bundle_root.display());
    }
    if !is_materialized_bundle(&bundle_root) {
        anyhow::bail!(
            "not a materialized bundle at {} (expected agent/manifest.json)",
            bundle_root.display()
        );
    }

    if !args.force {
        let agent_root = bundle_root.join("agent");
        let pkg = AgentPackage::load(&agent_root)
            .with_context(|| format!("load agent from {}", agent_root.display()))?;
        let fp = crate::commands::run::spec_fingerprint_for_package(&pkg);
        let base = args.control_plane.trim_end_matches('/');
        ensure_no_deployments_using_bundle(base, &pkg, &fp)
            .await
            .with_context(|| {
                format!(
                    "control plane at {base} unreachable; pass --force to remove without checking deployments"
                )
            })?;
    }

    std::fs::remove_dir_all(&bundle_root)
        .with_context(|| format!("remove {}", bundle_root.display()))?;
    println!("removed {}", bundle_root.display());
    Ok(())
}
