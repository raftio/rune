use crate::ToolContext;
use uuid::Uuid;

pub async fn post(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let title = input["title"].as_str().ok_or("Missing 'title' parameter")?;
    let description = input["description"].as_str().ok_or("Missing 'description' parameter")?;
    let assigned_to = input["assigned_to"].as_str();
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO task_queue (id, title, description, assigned_to, status, created_at) VALUES (?, ?, ?, ?, 'pending', strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
    )
    .bind(&id)
    .bind(title)
    .bind(description)
    .bind(assigned_to)
    .execute(&ctx.db)
    .await
    .map_err(|e| format!("Database error: {e}"))?;

    Ok(serde_json::json!({ "task_id": id, "message": format!("Task '{title}' created.") }))
}

pub async fn claim(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let agent_id = input["agent_id"].as_str().unwrap_or("unknown");

    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT id, title, description FROM task_queue WHERE status = 'pending' ORDER BY created_at ASC LIMIT 1",
    )
    .fetch_optional(&ctx.db)
    .await
    .map_err(|e| format!("Database error: {e}"))?;

    match row {
        Some((id, title, description)) => {
            sqlx::query(
                "UPDATE task_queue SET status = 'in_progress', claimed_by = ? WHERE id = ?",
            )
            .bind(agent_id)
            .bind(&id)
            .execute(&ctx.db)
            .await
            .map_err(|e| format!("Database error: {e}"))?;

            Ok(serde_json::json!({
                "task_id": id,
                "title": title,
                "description": description,
                "status": "in_progress",
            }))
        }
        None => Ok(serde_json::json!({ "message": "No tasks available." })),
    }
}

pub async fn complete(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let task_id = input["task_id"].as_str().ok_or("Missing 'task_id' parameter")?;
    let result = input["result"].as_str().ok_or("Missing 'result' parameter")?;

    let rows_affected = sqlx::query(
        "UPDATE task_queue SET status = 'completed', result = ?, completed_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?",
    )
    .bind(result)
    .bind(task_id)
    .execute(&ctx.db)
    .await
    .map_err(|e| format!("Database error: {e}"))?
    .rows_affected();

    if rows_affected == 0 {
        return Err(format!("Task '{task_id}' not found."));
    }

    Ok(serde_json::json!({ "message": format!("Task {task_id} marked as completed.") }))
}

pub async fn list(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let status = input["status"].as_str();

    let rows: Vec<(String, String, String, String, Option<String>, Option<String>)> = if let Some(s) = status {
        sqlx::query_as(
            "SELECT id, title, description, status, assigned_to, claimed_by FROM task_queue WHERE status = ? ORDER BY created_at DESC LIMIT 50",
        )
        .bind(s)
        .fetch_all(&ctx.db)
        .await
    } else {
        sqlx::query_as(
            "SELECT id, title, description, status, assigned_to, claimed_by FROM task_queue ORDER BY created_at DESC LIMIT 50",
        )
        .fetch_all(&ctx.db)
        .await
    }
    .map_err(|e| format!("Database error: {e}"))?;

    let tasks: Vec<serde_json::Value> = rows
        .iter()
        .map(|(id, title, desc, status, assigned, claimed)| {
            serde_json::json!({
                "id": id,
                "title": title,
                "description": desc,
                "status": status,
                "assigned_to": assigned,
                "claimed_by": claimed,
            })
        })
        .collect();

    Ok(serde_json::json!({ "count": tasks.len(), "tasks": tasks }))
}

pub async fn event_publish(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let event_type = input["event_type"].as_str().ok_or("Missing 'event_type' parameter")?;
    let payload = input.get("payload").cloned().unwrap_or(serde_json::json!({}));
    let payload_str = serde_json::to_string(&payload).unwrap_or_default();
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO events (id, event_type, payload, created_at) VALUES (?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
    )
    .bind(&id)
    .bind(event_type)
    .bind(&payload_str)
    .execute(&ctx.db)
    .await
    .map_err(|e| format!("Database error: {e}"))?;

    Ok(serde_json::json!({
        "event_id": id,
        "message": format!("Event '{event_type}' published.")
    }))
}
