use sqlx::SqlitePool;

use crate::error::StorageError;

pub async fn audit_decision(
    db: &SqlitePool,
    kind: &str,
    resource: &str,
    allowed: bool,
    reason: Option<&str>,
    request_id: Option<&str>,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO policy_audit (id, kind, resource, allowed, reason, request_id)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(kind)
    .bind(resource)
    .bind(if allowed { 1 } else { 0 })
    .bind(reason)
    .bind(request_id.unwrap_or(""))
    .execute(db)
    .await?;
    Ok(())
}
