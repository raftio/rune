use anyhow::{bail, Result};
use std::path::Path;

use crate::cli::{AgentInspectArgs, AgentLsArgs, AgentRmArgs, AgentSessionsArgs, AgentStopArgs};

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

/// Raw control-plane lists (deployments JSON rows, agent-version JSON rows).
pub(crate) async fn fetch_control_plane_state(
    http: &reqwest::Client,
    base: &str,
) -> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>)> {
    let body: serde_json::Value = http
        .get(format!("{base}/v1/deployments"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let deployments = body["deployments"].as_array().cloned().unwrap_or_default();

    let versions_value: serde_json::Value = http
        .get(format!("{base}/v1/agent-versions"))
        .send()
        .await?
        .json()
        .await
        .unwrap_or_default();

    let versions = versions_value.as_array().cloned().unwrap_or_default();

    Ok((deployments, versions))
}

fn attach_agent_names(
    deployments: Vec<serde_json::Value>,
    versions: &[serde_json::Value],
) -> Vec<(String, serde_json::Value)> {
    deployments
        .into_iter()
        .map(|d| {
            let vid = d["agent_version_id"].as_str().unwrap_or("");
            let name = find_agent_version(versions, vid)
                .and_then(|v| v["agent_name"].as_str())
                .unwrap_or("unknown")
                .to_string();
            (name, d)
        })
        .collect()
}

fn find_agent_version<'a>(
    versions: &'a [serde_json::Value],
    agent_version_id: &str,
) -> Option<&'a serde_json::Value> {
    versions
        .iter()
        .find(|v| v["id"].as_str() == Some(agent_version_id))
}

/// Fetch all deployments enriched with their agent name.
async fn fetch_named_deployments(
    http: &reqwest::Client,
    base: &str,
) -> Result<Vec<(String, serde_json::Value)>> {
    let (deployments, versions) = fetch_control_plane_state(http, base).await?;
    Ok(attach_agent_names(deployments, &versions))
}

pub async fn ls(args: AgentLsArgs) -> Result<()> {
    let http = reqwest::Client::new();
    let base = args.control_plane.trim_end_matches('/');
    let rows = fetch_named_deployments(&http, base).await?;

    if rows.is_empty() {
        println!("No agents deployed.");
        return Ok(());
    }

    println!(
        "{:<20} {:<12} {:<8} {:<10} {}",
        "AGENT", "ALIAS", "NS", "STATUS", "REPLICAS"
    );
    println!("{}", "-".repeat(60));

    for (name, d) in &rows {
        let alias = d["rollout_alias"].as_str().unwrap_or("-");
        let ns = d["namespace"].as_str().unwrap_or("?");
        let status = d["status"].as_str().unwrap_or("?");
        let desired = d["desired_replicas"].as_i64().unwrap_or(0);
        println!(
            "{:<20} {:<12} {:<8} {:<10} {}",
            name, alias, ns, status, desired
        );
    }

    Ok(())
}

pub async fn inspect(args: AgentInspectArgs) -> Result<()> {
    let http = reqwest::Client::new();
    let base = args.control_plane.trim_end_matches('/');
    let (deployments, versions) = fetch_control_plane_state(&http, base).await?;
    let rows = attach_agent_names(deployments, &versions);

    let matches: Vec<_> = rows
        .iter()
        .filter(|(name, d)| {
            name == &args.name
                && args
                    .ns
                    .as_deref()
                    .map_or(true, |ns| d["namespace"].as_str() == Some(ns))
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

        let vid = d["agent_version_id"].as_str().unwrap_or("");
        if let Some(v) = find_agent_version(&versions, vid) {
            let ver = v["version"].as_str().unwrap_or("?");
            let spec_sha = v["spec_sha256"].as_str().unwrap_or("?");
            let image_ref = v["image_ref"].as_str().unwrap_or("?");
            let image_digest = v["image_digest"].as_str().unwrap_or("?");
            let digest_short = if image_digest.len() > 12 {
                format!("{}…", &image_digest[..12])
            } else {
                image_digest.to_string()
            };
            let runtime_class = v["runtime_class"].as_str().unwrap_or("?");
            let vstatus = v["status"].as_str().unwrap_or("?");
            let vcreated = v["created_at"].as_str().unwrap_or("?");
            let vid_row = v["id"].as_str().unwrap_or("?");
            println!();
            println!("Registry / agent-version:");
            println!("  Version ID:   {vid_row}");
            println!("  Version:      {ver}");
            println!("  Spec SHA256:  {spec_sha}");
            println!("  Image:        {image_ref} ({digest_short})");
            println!("  Runtime:      {runtime_class}");
            println!("  Ver. status:  {vstatus}");
            println!("  Registered:   {}", &vcreated[..19.min(vcreated.len())]);
        } else {
            println!();
            println!("Registry / agent-version:");
            println!("  (no matching agent_version_id '{vid}' in /v1/agent-versions)");
        }
    }

    if let Some(ref dir) = args.agent_dir {
        println!();
        let spec_str = dir.to_string_lossy();
        match resolve_agent_source(&spec_str) {
            Ok((_tmp, pkg)) => {
                if pkg.spec.name != args.name {
                    eprintln!(
                        "warning: Runefile name '{}' does not match inspect target '{}'",
                        pkg.spec.name, args.name
                    );
                }
                println!("Local Runefile ({})", dir.display());
                if pkg.spec.skills.is_empty() {
                    println!("  skills:       (none)");
                } else {
                    println!("  skills:");
                    for s in &pkg.spec.skills {
                        println!("    - {s}");
                    }
                }
                if pkg.spec.toolset.is_empty() {
                    println!("  toolset:      (none)");
                } else {
                    println!("  toolset:");
                    let max_tools = 50;
                    for (i, t) in pkg.spec.toolset.iter().enumerate() {
                        if i >= max_tools {
                            println!("    … and {} more", pkg.spec.toolset.len() - max_tools);
                            break;
                        }
                        println!("    - {t}");
                    }
                }
                let instr = pkg.spec.instructions.trim();
                let excerpt_len = 500usize;
                if instr.len() <= excerpt_len {
                    println!("  instructions:");
                    for line in instr.lines() {
                        println!("    {line}");
                    }
                } else {
                    let excerpt = &instr[..excerpt_len];
                    println!("  instructions (excerpt):");
                    for line in excerpt.lines() {
                        println!("    {line}");
                    }
                    println!("    …");
                }
            }
            Err(e) => {
                eprintln!("warning: could not load --agent-dir: {e}");
            }
        }
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
                && args
                    .ns
                    .as_deref()
                    .map_or(true, |ns| d["namespace"].as_str() == Some(ns))
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

    let session_id =
        session_id.ok_or_else(|| anyhow::anyhow!("agent '{}' has no sessions yet", args.name))?;

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
                && args
                    .ns
                    .as_deref()
                    .map_or(true, |ns| d["namespace"].as_str() == Some(ns))
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
                && args
                    .ns
                    .as_deref()
                    .map_or(true, |ns| d["namespace"].as_str() == Some(ns))
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

#[cfg(test)]
mod inspect_tests {
    use super::{attach_agent_names, find_agent_version};
    use serde_json::json;

    #[test]
    fn find_agent_version_matches_id() {
        let versions = vec![
            json!({"id": "a", "agent_name": "n1"}),
            json!({"id": "b", "version": "0.1.0", "spec_sha256": "abc"}),
        ];
        let v = find_agent_version(&versions, "b").unwrap();
        assert_eq!(v["version"], "0.1.0");
        assert!(find_agent_version(&versions, "missing").is_none());
    }

    #[test]
    fn attach_agent_names_maps_deployments() {
        let versions = vec![json!({"id": "vid-1", "agent_name": "chat"})];
        let depls = vec![json!({"agent_version_id": "vid-1", "namespace": "dev"})];
        let rows = attach_agent_names(depls, &versions);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "chat");
    }
}
