use anyhow::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;

use crate::cli::SessionsArgs;

/// Resolve id to session_id: try as session first, then as deployment (latest session).
async fn resolve_session_id(
    db: &sqlx::sqlite::SqlitePool,
    id: &str,
) -> Result<String, String> {
    // 1. Try as session_id
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions WHERE id = ?")
        .bind(id)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())?;
    if exists > 0 {
        return Ok(id.to_string());
    }

    // 2. Try as deployment_id -> latest session
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM agent_sessions WHERE deployment_id = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())?;
    if let Some((session_id,)) = row {
        return Ok(session_id);
    }

    // 3. Check if deployment exists (no sessions yet)
    let dep_exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_deployments WHERE id = ?")
            .bind(id)
            .fetch_one(db)
            .await
            .map_err(|e| e.to_string())?;
    if dep_exists > 0 {
        return Err("deployment has no sessions yet (no requests have been made)".to_string());
    }

    Err(format!("'{}' not found as session or deployment", id))
}

pub async fn exec(args: SessionsArgs) -> Result<()> {
    let opts = SqliteConnectOptions::from_str(&args.database_url)?.read_only(true);
    let db = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;

    let session_id = resolve_session_id(&db, &args.id)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    #[derive(sqlx::FromRow)]
    struct Row {
        role: String,
        content: String,
        created_at: String,
    }

    let rows = sqlx::query_as::<_, Row>(
        "SELECT role, content, created_at FROM session_messages
         WHERE session_id = ? ORDER BY created_at ASC",
    )
    .bind(&session_id)
    .fetch_all(&db)
    .await?;

    println!("Session: {}", session_id);
    println!("{}", "─".repeat(60));

    if rows.is_empty() {
        println!("(no messages)");
        return Ok(());
    }

    for row in &rows {
        let content: serde_json::Value = serde_json::from_str(&row.content)
            .unwrap_or(serde_json::Value::String(row.content.clone()));
        let text = blocks_to_text(&content);
        let ts = &row.created_at[..19];
        let label = row.role.to_uppercase();
        println!("[{ts}] {label}");
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
