use anyhow::{bail, Result};
use std::path::Path;

use crate::cli::{AgentInspectArgs, AgentSessionsArgs, AgentLsArgs, AgentRmArgs, AgentStopArgs};

/// Resolve an agent source to an `AgentPackage`.
///
/// Returns `(Option<TempDir>, AgentPackage)` — the `TempDir` handle must be
/// kept alive for the lifetime of any paths derived from the cloned repo.
pub fn resolve_agent_source(
    agent_spec: &str,
) -> Result<(Option<tempfile::TempDir>, rune_spec::AgentPackage)> {
    use rune_spec::AgentPackage;

    if let Some(rest) = agent_spec.strip_prefix("git://") {
        let (repo_url, subdir) = match rest.split_once('#') {
            Some((url, fragment)) => (format!("https://{url}"), Some(fragment)),
            None => (format!("https://{rest}"), None),
        };

        let tmp = tempfile::tempdir()?;

        let status = std::process::Command::new("git")
            .args(["clone", "--depth", "1", &repo_url])
            .arg(tmp.path())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()?;

        if !status.success() {
            bail!("git clone failed for {repo_url}");
        }

        let agent_dir = match subdir {
            Some(sub) => tmp.path().join(sub),
            None => tmp.path().to_path_buf(),
        };

        let pkg = AgentPackage::load(&agent_dir)?;
        Ok((Some(tmp), pkg))
    } else {
        let path = Path::new(agent_spec);
        let agent_dir = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        let pkg = AgentPackage::load(agent_dir)?;
        Ok((None, pkg))
    }
}

// ---------------------------------------------------------------------------
// Runtime management helpers
// ---------------------------------------------------------------------------

/// Fetch all deployments enriched with their agent name.
async fn fetch_named_deployments(
    http: &reqwest::Client,
    base: &str,
) -> Result<Vec<(String, serde_json::Value)>> {
    let body: serde_json::Value = http
        .get(format!("{base}/v1/deployments"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let deployments = body["deployments"].as_array().cloned().unwrap_or_default();

    let versions: serde_json::Value = http
        .get(format!("{base}/v1/agent-versions"))
        .send()
        .await?
        .json()
        .await
        .unwrap_or_default();

    let result = deployments
        .into_iter()
        .map(|d| {
            let vid = d["agent_version_id"].as_str().unwrap_or("");
            let name = versions
                .as_array()
                .and_then(|arr| arr.iter().find(|v| v["id"].as_str() == Some(vid)))
                .and_then(|v| v["agent_name"].as_str())
                .unwrap_or("unknown")
                .to_string();
            (name, d)
        })
        .collect();

    Ok(result)
}

pub async fn ls(args: AgentLsArgs) -> Result<()> {
    let http = reqwest::Client::new();
    let base = args.control_plane.trim_end_matches('/');
    let rows = fetch_named_deployments(&http, base).await?;

    if rows.is_empty() {
        println!("No agents deployed.");
        return Ok(());
    }

    println!("{:<20} {:<12} {:<8} {:<10} {}", "AGENT", "ALIAS", "NS", "STATUS", "REPLICAS");
    println!("{}", "-".repeat(60));

    for (name, d) in &rows {
        let alias = d["rollout_alias"].as_str().unwrap_or("-");
        let ns = d["namespace"].as_str().unwrap_or("?");
        let status = d["status"].as_str().unwrap_or("?");
        let desired = d["desired_replicas"].as_i64().unwrap_or(0);
        println!("{:<20} {:<12} {:<8} {:<10} {}", name, alias, ns, status, desired);
    }

    Ok(())
}

pub async fn inspect(args: AgentInspectArgs) -> Result<()> {
    let http = reqwest::Client::new();
    let base = args.control_plane.trim_end_matches('/');
    let rows = fetch_named_deployments(&http, base).await?;

    let matches: Vec<_> = rows
        .iter()
        .filter(|(name, d)| {
            name == &args.name
                && args.ns.as_deref().map_or(true, |ns| d["namespace"].as_str() == Some(ns))
                && args
                    .alias
                    .as_deref()
                    .map_or(true, |a| d["rollout_alias"].as_str() == Some(a))
        })
        .collect();

    if matches.is_empty() {
        bail!("no deployed agent found matching name '{}'", args.name);
    }

    for (i, (name, d)) in matches.iter().enumerate() {
        if i > 0 {
            println!();
        }
        let alias = d["rollout_alias"].as_str().unwrap_or("-");
        let ns = d["namespace"].as_str().unwrap_or("?");
        let status = d["status"].as_str().unwrap_or("?");
        let desired = d["desired_replicas"].as_i64().unwrap_or(0);
        let min = d["min_replicas"].as_i64().unwrap_or(0);
        let max = d["max_replicas"].as_i64().unwrap_or(0);
        let concurrency = d["concurrency_limit"].as_i64().unwrap_or(0);
        let id = d["id"].as_str().unwrap_or("?");
        let created = d["created_at"].as_str().unwrap_or("?");

        println!("Agent:       {name}");
        println!("Alias:       {alias}");
        println!("Namespace:   {ns}");
        println!("Status:      {status}");
        println!("Replicas:    {desired}  (min: {min}, max: {max})");
        println!("Concurrency: {concurrency}");
        println!("Created:     {}", &created[..19.min(created.len())]);
        println!("ID:          {id}");
    }

    Ok(())
}

pub async fn sessions_by_name(args: AgentSessionsArgs) -> Result<()> {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    let http = reqwest::Client::new();
    let base = args.control_plane.trim_end_matches('/');
    let rows = fetch_named_deployments(&http, base).await?;

    let deployment = rows
        .iter()
        .find(|(name, d)| {
            name == &args.name
                && args.ns.as_deref().map_or(true, |ns| d["namespace"].as_str() == Some(ns))
                && args
                    .alias
                    .as_deref()
                    .map_or(true, |a| d["rollout_alias"].as_str() == Some(a))
        })
        .map(|(_, d)| d)
        .ok_or_else(|| anyhow::anyhow!("no deployed agent found matching name '{}'", args.name))?;

    let deployment_id = deployment["id"].as_str().unwrap_or("?");

    let opts = SqliteConnectOptions::from_str(&args.database_url)?.read_only(true);
    let db = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;

    let session_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM agent_sessions WHERE deployment_id = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(deployment_id)
    .fetch_optional(&db)
    .await?;

    let session_id = session_id
        .ok_or_else(|| anyhow::anyhow!("agent '{}' has no sessions yet", args.name))?;

    #[derive(sqlx::FromRow)]
    struct Row {
        role: String,
        content: String,
        created_at: String,
    }

    let messages = sqlx::query_as::<_, Row>(
        "SELECT role, content, created_at FROM session_messages
         WHERE session_id = ? ORDER BY created_at ASC",
    )
    .bind(&session_id)
    .fetch_all(&db)
    .await?;

    println!("Agent:   {}", args.name);
    println!("Session: {session_id}");
    println!("{}", "─".repeat(60));

    if messages.is_empty() {
        println!("(no messages)");
        return Ok(());
    }

    for row in &messages {
        let content: serde_json::Value = serde_json::from_str(&row.content)
            .unwrap_or(serde_json::Value::String(row.content.clone()));
        let text = blocks_to_text(&content);
        let ts = &row.created_at[..19.min(row.created_at.len())];
        println!("[{ts}] {}", row.role.to_uppercase());
        println!("{text}");
        println!();
    }

    Ok(())
}

fn blocks_to_text(content: &serde_json::Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        let parts: Vec<String> = arr
            .iter()
            .filter_map(|b| match b["type"].as_str() {
                Some("text") => b["text"].as_str().map(str::to_string),
                Some("tool_use") => {
                    Some(format!("[tool_use: {}]", b["name"].as_str().unwrap_or("?")))
                }
                Some("tool_result") => Some(format!(
                    "[tool_result for: {}]",
                    b["tool_use_id"].as_str().unwrap_or("?")
                )),
                _ => None,
            })
            .collect();
        if !parts.is_empty() {
            return parts.join("\n");
        }
    }
    content.to_string()
}

pub async fn stop_by_name(args: AgentStopArgs) -> Result<()> {
    let http = reqwest::Client::new();
    let base = args.control_plane.trim_end_matches('/');
    let rows = fetch_named_deployments(&http, base).await?;

    let matches: Vec<_> = rows
        .iter()
        .filter(|(name, d)| {
            name == &args.name
                && args.ns.as_deref().map_or(true, |ns| d["namespace"].as_str() == Some(ns))
                && args
                    .alias
                    .as_deref()
                    .map_or(true, |a| d["rollout_alias"].as_str() == Some(a))
        })
        .collect();

    if matches.is_empty() {
        bail!("no deployed agent found matching name '{}'", args.name);
    }

    for (_, d) in &matches {
        let id = d["id"].as_str().unwrap_or("?");
        let ns = d["namespace"].as_str().unwrap_or("?");
        let alias = d["rollout_alias"].as_str().unwrap_or("-");
        http.post(format!("{base}/v1/deployments/{id}/scale"))
            .json(&serde_json::json!({ "desired_replicas": 0 }))
            .send()
            .await?
            .error_for_status()?;
        println!("stopped: {} (alias={} ns={})", args.name, alias, ns);
    }

    Ok(())
}

pub async fn rm_by_name(args: AgentRmArgs) -> Result<()> {
    let http = reqwest::Client::new();
    let base = args.control_plane.trim_end_matches('/');
    let rows = fetch_named_deployments(&http, base).await?;

    let matches: Vec<_> = rows
        .iter()
        .filter(|(name, d)| {
            name == &args.name
                && args.ns.as_deref().map_or(true, |ns| d["namespace"].as_str() == Some(ns))
                && args
                    .alias
                    .as_deref()
                    .map_or(true, |a| d["rollout_alias"].as_str() == Some(a))
        })
        .collect();

    if matches.is_empty() {
        bail!("no deployed agent found matching name '{}'", args.name);
    }

    for (_, d) in &matches {
        let id = d["id"].as_str().unwrap_or("?");
        let ns = d["namespace"].as_str().unwrap_or("?");
        let alias = d["rollout_alias"].as_str().unwrap_or("-");
        let url = if args.force {
            format!("{base}/v1/deployments/{id}?force=true")
        } else {
            format!("{base}/v1/deployments/{id}")
        };
        let resp = http.delete(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!(
                "failed to remove {} (alias={} ns={}): {} {}",
                args.name,
                alias,
                ns,
                status,
                body
            );
        }
        println!("removed: {} (alias={} ns={})", args.name, alias, ns);
    }

    Ok(())
}

