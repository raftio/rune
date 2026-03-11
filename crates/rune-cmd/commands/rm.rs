use anyhow::{bail, Result};
use uuid::Uuid;

use crate::cli::RmAgentArgs;

pub async fn exec(args: RmAgentArgs) -> Result<()> {
    let id: Uuid = args
        .deployment_id
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid deployment ID: {}", args.deployment_id))?;

    let http = reqwest::Client::new();
    let base = args.control_plane.trim_end_matches('/');
    let url = if args.force {
        format!("{base}/v1/deployments/{id}?force=true")
    } else {
        format!("{base}/v1/deployments/{id}")
    };

    let resp = http.delete(&url).send().await?;

    if resp.status() == 404 {
        bail!("Deployment {} not found", id);
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let hint = if args.force && status.as_u16() == 500 {
            "\n  Hint: Restart the daemon (rune daemon stop; rune daemon start) to pick up --force support."
        } else {
            ""
        };
        bail!(
            "failed to remove deployment {}: {} {}{}",
            id,
            status,
            if body.is_empty() { "".into() } else { format!("— {}", body) },
            hint,
        );
    }

    println!("Deployment {} removed", id);
    Ok(())
}
