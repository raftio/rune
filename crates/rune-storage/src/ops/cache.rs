use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::StorageError;

const REPLICA_LOAD_NS: &str = "replica_load";

pub async fn get_replica_load(
    db: &SqlitePool,
    replica_id: Uuid,
) -> Result<Option<i32>, StorageError> {
    let row: Option<String> = sqlx::query_scalar(
        "SELECT value FROM rune_cache WHERE namespace = ? AND key = ?",
    )
    .bind(REPLICA_LOAD_NS)
    .bind(replica_id.to_string())
    .fetch_optional(db)
    .await?;
    Ok(row.and_then(|s| s.parse().ok()))
}

pub async fn set_replica_load(
    db: &SqlitePool,
    replica_id: Uuid,
    load: i32,
) -> Result<(), StorageError> {
    let expires_at = (Utc::now() + chrono::Duration::seconds(60)).to_rfc3339();
    sqlx::query(
        "INSERT OR REPLACE INTO rune_cache (namespace, key, value, expires_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(REPLICA_LOAD_NS)
    .bind(replica_id.to_string())
    .bind(load.to_string())
    .bind(&expires_at)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn rate_limit_check(
    db: &SqlitePool,
    key: &str,
    window_seconds: i64,
    max_count: i64,
) -> Result<bool, StorageError> {
    let now = Utc::now();
    let cutoff = (now - chrono::Duration::seconds(window_seconds)).to_rfc3339();

    #[derive(sqlx::FromRow)]
    struct Row {
        count: i64,
        window_start: String,
    }

    let row: Option<Row> = sqlx::query_as(
        "SELECT count, window_start FROM rune_rate_limits WHERE key = ?",
    )
    .bind(key)
    .fetch_optional(db)
    .await?;

    let (count, window_start) = match row {
        Some(r) if r.window_start >= cutoff => (r.count, r.window_start),
        _ => (0i64, now.to_rfc3339()),
    };

    if count >= max_count {
        return Ok(false);
    }

    let now_str = now.to_rfc3339();
    sqlx::query(
        r#"INSERT INTO rune_rate_limits (key, window_start, count, window_seconds)
           VALUES (?, ?, 1, ?)
           ON CONFLICT(key) DO UPDATE SET
              count = CASE WHEN window_start < ? THEN 1 ELSE count + 1 END,
              window_start = CASE WHEN window_start < ? THEN ? ELSE window_start END"#,
    )
    .bind(key)
    .bind(&window_start)
    .bind(window_seconds)
    .bind(&cutoff)
    .bind(&cutoff)
    .bind(&now_str)
    .execute(db)
    .await?;

    Ok(true)
}

pub async fn cleanup_expired(db: &SqlitePool) -> Result<u64, StorageError> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query("DELETE FROM rune_cache WHERE expires_at < ?")
        .bind(&now)
        .execute(db)
        .await?;
    Ok(result.rows_affected())
}
