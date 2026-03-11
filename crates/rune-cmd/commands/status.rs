use anyhow::Result;

use crate::cli::StatusArgs;

pub async fn exec(args: StatusArgs) -> Result<()> {
    let http = reqwest::Client::new();
    let base = args.control_plane.trim_end_matches('/');

    let body: serde_json::Value = http
        .get(format!("{base}/v1/deployments"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let deployments = body["deployments"].as_array().cloned().unwrap_or_default();

    if deployments.is_empty() {
        println!("No deployments found.");
        return Ok(());
    }

    let versions: serde_json::Value = http
        .get(format!("{base}/v1/agent-versions"))
        .send()
        .await?
        .json()
        .await
        .unwrap_or_default();

    println!(
        "{:<20} {:<12} {:<8} {:<10} {}",
        "AGENT", "ALIAS", "NS", "STATUS", "REPLICAS"
    );
    println!("{}", "-".repeat(60));

    for d in &deployments {
        let ns = d["namespace"].as_str().unwrap_or("?");
        let status = d["status"].as_str().unwrap_or("?");
        let desired = d["desired_replicas"].as_i64().unwrap_or(0);
        let alias = d["rollout_alias"].as_str().unwrap_or("-");
        let vid = d["agent_version_id"].as_str().unwrap_or("?");

        let agent = versions
            .as_array()
            .and_then(|arr| arr.iter().find(|v| v["id"].as_str() == Some(vid)))
            .and_then(|v| v["agent_name"].as_str())
            .unwrap_or("unknown");

        println!("{:<20} {:<12} {:<8} {:<10} {}", agent, alias, ns, status, desired);
    }
    Ok(())
}
