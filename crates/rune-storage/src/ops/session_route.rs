use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::StorageError;

const SESSION_ROUTE_NS: &str = "session_route";
const CACHE_TTL_SECS: i64 = 300;

pub async fn get(db: &SqlitePool, session_id: Uuid) -> Result<Option<Uuid>, StorageError> {
    let now = Utc::now().to_rfc3339();
    let row: Option<String> = sqlx::query_scalar(
        "SELECT value FROM rune_cache
         WHERE namespace = ? AND key = ? AND expires_at > ?",
    )
    .bind(SESSION_ROUTE_NS)
    .bind(session_id.to_string())
    .bind(&now)
    .fetch_optional(db)
    .await?;
    Ok(row.and_then(|s| s.parse().ok()))
}

pub async fn set(
    db: &SqlitePool,
    session_id: Uuid,
    replica_id: Uuid,
    ttl_secs: Option<i64>,
) -> Result<(), StorageError> {
    let ttl = ttl_secs.unwrap_or(CACHE_TTL_SECS);
    let expires_at = (Utc::now() + chrono::Duration::seconds(ttl)).to_rfc3339();
    sqlx::query(
        "INSERT OR REPLACE INTO rune_cache (namespace, key, value, expires_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(SESSION_ROUTE_NS)
    .bind(session_id.to_string())
    .bind(replica_id.to_string())
    .bind(&expires_at)
    .execute(db)
    .await?;
    Ok(())
}
