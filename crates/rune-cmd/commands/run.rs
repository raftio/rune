use anyhow::Result;
use std::hash::{DefaultHasher, Hash, Hasher};

use crate::cli::RunArgs;
use super::agent::resolve_agent_source;

pub async fn exec(args: RunArgs) -> Result<()> {
    let (_tmp, pkg) = resolve_agent_source(&args.agent_spec)?;
    println!("Deploying {} v{} ...", pkg.spec.name, pkg.spec.version);

    let http = reqwest::Client::new();
    let base = args.control_plane.trim_end_matches('/');

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
    println!("  registered version: {version_id}");

    let resp = http
        .post(format!("{base}/v1/deployments"))
        .json(&serde_json::json!({
            "agent_version_id": version_id,
            "namespace":        args.namespace,
            "rollout_alias":    args.alias,
        }))
        .send()
        .await?
        .error_for_status()?;

    let deploy: serde_json::Value = resp.json().await?;
    let deploy_id = deploy["id"].as_str().unwrap_or("?");
    println!("  created deployment: {deploy_id}");
    println!(
        "  status: {} (reconcile loop activates within ~10s)",
        deploy["status"].as_str().unwrap_or("?")
    );
    println!("Done. Agent '{}' deployed.", pkg.spec.name);
    Ok(())
}

fn simple_hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}
