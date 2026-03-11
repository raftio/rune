use anyhow::{bail, Result};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

use rune_spec::ComposeSpec;

use crate::cli::{ComposeDownArgs, ComposePsArgs, ComposeUpArgs};
use super::agent::resolve_agent_source;

pub async fn up(args: ComposeUpArgs) -> Result<()> {
    let compose = ComposeSpec::load(Path::new(&args.file))?;
    let order = compose
        .topological_order()
        .expect("validation ensures acyclic graph");

    let http = reqwest::Client::new();
    let base = args.control_plane.trim_end_matches('/');

    println!();
    println!(
        "  \x1b[32m▲\x1b[0m \x1b[1mCompose up\x1b[0m — project \x1b[36m{}\x1b[0m ({} agents)",
        compose.project,
        order.len(),
    );
    println!();

    let mut _temps = Vec::new();

    for name in &order {
        let entry = &compose.agents[name];

        let (tmp, pkg) = resolve_agent_source(&entry.source)?;
        _temps.push(tmp);

        println!(
            "  \x1b[33m●\x1b[0m Deploying \x1b[1m{name}\x1b[0m ({} v{}) ...",
            pkg.spec.name, pkg.spec.version,
        );

        let resp = http
            .post(format!("{base}/v1/agent-versions"))
            .json(&serde_json::json!({
                "agent_name":    pkg.spec.name,
                "version":       pkg.spec.version,
                "image_ref":     format!("local/{}", pkg.spec.name),
                "image_digest":  format!("sha256:{:x}", simple_hash(&pkg.spec.name)),
                "spec_sha256":   format!("{:x}", simple_hash(&pkg.spec.instructions)),
                "runtime_class": "wasm",
            }))
            .send()
            .await?
            .error_for_status()?;

        let version: serde_json::Value = resp.json().await?;
        let version_id = version["id"].as_str().unwrap_or("?");

        let resolved_env = compose.resolved_env(name);
        let resp = http
            .post(format!("{base}/v1/deployments"))
            .json(&serde_json::json!({
                "agent_version_id": version_id,
                "namespace":        entry.namespace,
                "rollout_alias":    entry.alias,
                "desired_replicas": entry.replicas,
                "project_ref":      compose.project,
                "env":              resolved_env,
            }))
            .send()
            .await?
            .error_for_status()?;

        let deploy: serde_json::Value = resp.json().await?;
        let deploy_id = deploy["id"].as_str().unwrap_or("?");
        println!(
            "    version: {version_id}  deployment: {deploy_id}  status: {}",
            deploy["status"].as_str().unwrap_or("?"),
        );
    }

    println!();
    println!(
        "  \x1b[32m▲\x1b[0m \x1b[1mAll agents deployed\x1b[0m (reconcile loop activates within ~10s)",
    );
    println!();
    Ok(())
}

pub async fn down(args: ComposeDownArgs) -> Result<()> {
    let compose = ComposeSpec::load(Path::new(&args.file))?;
    let http = reqwest::Client::new();
    let base = args.control_plane.trim_end_matches('/');

    println!();
    println!(
        "  \x1b[33m●\x1b[0m \x1b[1mCompose down\x1b[0m — project \x1b[36m{}\x1b[0m",
        compose.project,
    );

    let body: serde_json::Value = http
        .get(format!("{base}/v1/deployments?project={}", compose.project))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let deployments = body["deployments"].as_array().cloned().unwrap_or_default();

    if deployments.is_empty() {
        println!("  No deployments found for project '{}'.", compose.project);
        println!();
        return Ok(());
    }

    for d in &deployments {
        let id = d["id"].as_str().unwrap_or("?");
        let status = d["status"].as_str().unwrap_or("?");

        println!("  \x1b[31m✕\x1b[0m Removing deployment {id} (status: {status}) ...");

        let resp = http
            .delete(format!("{base}/v1/deployments/{id}"))
            .send()
            .await?;

        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NOT_FOUND {
            let code = resp.status();
            let msg = resp.text().await.unwrap_or_default();
            bail!("failed to delete deployment {id}: {code} {msg}");
        }
    }

    println!();
    println!(
        "  \x1b[32m▲\x1b[0m \x1b[1m{} deployment(s) removed\x1b[0m",
        deployments.len(),
    );
    println!();
    Ok(())
}

pub async fn ps(args: ComposePsArgs) -> Result<()> {
    let compose = ComposeSpec::load(Path::new(&args.file))?;
    let http = reqwest::Client::new();
    let base = args.control_plane.trim_end_matches('/');

    let body: serde_json::Value = http
        .get(format!("{base}/v1/deployments?project={}", compose.project))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let deployments = body["deployments"].as_array().cloned().unwrap_or_default();

    if deployments.is_empty() {
        println!("No deployments found for project '{}'.", compose.project);
        return Ok(());
    }

    let versions: serde_json::Value = http
        .get(format!("{base}/v1/agent-versions"))
        .send()
        .await?
        .json()
        .await
        .unwrap_or_default();

    println!();
    println!(
        "  Project: \x1b[36m{}\x1b[0m",
        compose.project,
    );
    println!();
    println!(
        "  {:<38} {:<20} {:<8} {:<10} {:<8} REPLICAS",
        "DEPLOYMENT ID", "AGENT", "NS", "STATUS", "ALIAS",
    );
    println!("  {}", "-".repeat(96));

    for d in &deployments {
        let id = d["id"].as_str().unwrap_or("?");
        let ns = d["namespace"].as_str().unwrap_or("?");
        let status = d["status"].as_str().unwrap_or("?");
        let alias = d["rollout_alias"].as_str().unwrap_or("?");
        let replicas = d["desired_replicas"].as_i64().unwrap_or(0);
        let vid = d["agent_version_id"].as_str().unwrap_or("?");

        let agent = versions
            .as_array()
            .and_then(|arr| arr.iter().find(|v| v["id"].as_str() == Some(vid)))
            .and_then(|v| v["agent_name"].as_str())
            .unwrap_or("unknown");

        println!(
            "  {:<38} {:<20} {:<8} {:<10} {:<8} {}",
            id, agent, ns, status, alias, replicas,
        );
    }
    println!();
    Ok(())
}

fn simple_hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}
