use anyhow::{bail, Result};
use uuid::Uuid;

use crate::cli::StopAgentArgs;

pub async fn exec(args: StopAgentArgs) -> Result<()> {
    let id: Uuid = args
        .deployment_id
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid deployment ID: {}", args.deployment_id))?;

    let http = reqwest::Client::new();
    let base = args.control_plane.trim_end_matches('/');
    let url = format!("{base}/v1/deployments/{id}/scale");

    let resp = http
        .post(&url)
        .json(&serde_json::json!({ "desired_replicas": 0 }))
        .send()
        .await?;

    if resp.status() == 404 {
        bail!("Deployment {} not found", id);
    }
    resp.error_for_status()?;

    println!("Deployment {} stopped (scaled to 0)", id);
    Ok(())
}
