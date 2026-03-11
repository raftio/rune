use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::StorageError;

pub async fn insert(
    db: &SqlitePool,
    request_id: Uuid,
    session_id: Uuid,
    deployment_id: Uuid,
    replica_id: Option<Uuid>,
    payload: &serde_json::Value,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO agent_requests (id, session_id, deployment_id, replica_id, request_payload, status)
         VALUES (?, ?, ?, ?, ?, 'running')",
    )
    .bind(request_id.to_string())
    .bind(session_id.to_string())
    .bind(deployment_id.to_string())
    .bind(replica_id.map(|id| id.to_string()))
    .bind(serde_json::to_string(payload).unwrap_or_default())
    .execute(db)
    .await?;
    Ok(())
}

pub async fn update_completed(
    db: &SqlitePool,
    request_id: Uuid,
    output: &serde_json::Value,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE agent_requests
         SET status = 'completed',
             response_summary = ?,
             completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?",
    )
    .bind(serde_json::to_string(output).unwrap_or_default())
    .bind(request_id.to_string())
    .execute(db)
    .await?;
    Ok(())
}

pub async fn update_status(
    db: &SqlitePool,
    request_id: Uuid,
    status: &str,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE agent_requests
         SET status = ?,
             completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?",
    )
    .bind(status)
    .bind(request_id.to_string())
    .execute(db)
    .await?;
    Ok(())
}

pub async fn get_response_summary(
    db: &SqlitePool,
    request_id: &str,
) -> Result<Option<String>, StorageError> {
    let summary: Option<String> = sqlx::query_scalar(
        "SELECT response_summary FROM agent_requests WHERE id = ?",
    )
    .bind(request_id)
    .fetch_optional(db)
    .await?;
    Ok(summary)
}
