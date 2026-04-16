use std::fs::File;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rune_artifact::PackOptions;
use rune_spec::AgentPackage;

use crate::cli::{ArtifactBuildArgs, ArtifactVerifyArgs};

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

/// ~/.rune/artifacts/{name}-{tag}.tar.gz (sanitized components)
fn artifact_path(agent_name: &str, tag: &str) -> Result<PathBuf> {
    let base = dirs::home_dir()
        .context("could not resolve home directory")?
        .join(".rune")
        .join("artifacts");
    let name = sanitize_filename_component(agent_name);
    let tag = sanitize_filename_component(tag);
    Ok(base.join(format!("{name}-{tag}.tar.gz")))
}

fn prepare_artifact_output_path(agent_name: &str, tag: &str) -> Result<PathBuf> {
    let p = artifact_path(agent_name, tag)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(p)
}

pub fn build(args: ArtifactBuildArgs) -> Result<()> {
    let agent_dir = args.agent_dir;
    let tag = args.tag.unwrap_or_else(|| "latest".to_string());

    let pkg = AgentPackage::load(&agent_dir)
        .with_context(|| format!("load agent from {}", agent_dir.display()))?;

    let output = prepare_artifact_output_path(&pkg.spec.name, &tag)?;

    let summary = rune_artifact::pack_agent_dir_to_file(
        &agent_dir,
        &output,
        PackOptions {
            tag: Some(tag.clone()),
        },
    )?;
    println!("artifact_sha256={}", summary.artifact_sha256);
    println!("tag={tag}");
    println!("{}", output.display());
    Ok(())
}

pub fn verify_cmd(args: ArtifactVerifyArgs) -> Result<()> {
    let path = artifact_path(&args.name, &args.tag)?;
    let f = File::open(&path)
        .with_context(|| format!("open artifact {}", path.display()))?;
    let m = rune_artifact::verify(f)?;
    println!("format: {}", m.format);
    if let Some(ref i) = m.initiative {
        println!("initiative: {i}");
    }
    if let Some(ref t) = m.tag {
        println!("tag: {t}");
    }
    println!("agent_name: {}", m.agent_name);
    println!("agent_version: {}", m.agent_version);
    println!("created_at: {}", m.created_at);
    println!("files: {}", m.files.len());
    Ok(())
}
