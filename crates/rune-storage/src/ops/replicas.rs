use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{NewReplica, ReplicaState};

pub async fn count_healthy(
    db: &SqlitePool,
    deployment_id: Uuid,
) -> Result<i64, StorageError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_replicas WHERE deployment_id = ? AND state IN ('starting', 'ready')",
    )
    .bind(deployment_id.to_string())
    .fetch_one(db)
    .await?;
    Ok(count)
}

pub async fn insert(db: &SqlitePool, r: &NewReplica) -> Result<Uuid, StorageError> {
    // Defensive: avoid FK violation when InsertReplica is applied after DeleteDeployment
    // (e.g. compose down committed before reconcile's InsertReplica).
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_deployments WHERE id = ?)",
    )
    .bind(r.deployment_id.to_string())
    .fetch_one(db)
    .await?;
    if !exists {
        return Err(StorageError::NotFound(format!(
            "deployment {} no longer exists (stale InsertReplica, e.g. compose down)",
            r.deployment_id
        )));
    }

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_replicas
            (id, deployment_id, backend_type, backend_instance_id,
             state, concurrency_limit, current_load)
         VALUES (?, ?, ?, ?, 'starting', ?, 0)",
    )
    .bind(id.to_string())
    .bind(r.deployment_id.to_string())
    .bind(r.backend_type.to_string())
    .bind(&r.backend_instance_id)
    .bind(r.concurrency_limit)
    .execute(db)
    .await?;
    Ok(id)
}

pub async fn set_ready(db: &SqlitePool, id: Uuid) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE agent_replicas
         SET state = 'ready',
             started_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             last_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?",
    )
    .bind(id.to_string())
    .execute(db)
    .await?;
    Ok(())
}

pub async fn set_state(
    db: &SqlitePool,
    id: Uuid,
    state: ReplicaState,
) -> Result<(), StorageError> {
    sqlx::query("UPDATE agent_replicas SET state = ? WHERE id = ?")
        .bind(state.to_string())
        .bind(id.to_string())
        .execute(db)
        .await?;
    Ok(())
}

pub async fn heartbeat(db: &SqlitePool, id: Uuid) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE agent_replicas SET last_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
    )
    .bind(id.to_string())
    .execute(db)
    .await?;
    Ok(())
}

pub async fn get_drainable_ids(
    db: &SqlitePool,
    deployment_id: Uuid,
    limit: i64,
) -> Result<Vec<Uuid>, StorageError> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM agent_replicas
         WHERE deployment_id = ? AND state = 'ready'
         ORDER BY current_load ASC
         LIMIT ?",
    )
    .bind(deployment_id.to_string())
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(ids.into_iter().filter_map(|s| s.parse().ok()).collect())
}

pub async fn mark_stale_failed(
    db: &SqlitePool,
    threshold_secs: i64,
) -> Result<u64, StorageError> {
    let result = sqlx::query(
        "UPDATE agent_replicas
         SET state = 'failed'
         WHERE state IN ('starting', 'ready')
           AND last_heartbeat_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-' || ? || ' seconds')",
    )
    .bind(threshold_secs)
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

pub async fn select_least_loaded(
    db: &SqlitePool,
    deployment_id: Uuid,
) -> Result<Option<Uuid>, StorageError> {
    let id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM agent_replicas
         WHERE deployment_id = ? AND state = 'ready'
           AND current_load < concurrency_limit
         ORDER BY current_load ASC
         LIMIT 1",
    )
    .bind(deployment_id.to_string())
    .fetch_optional(db)
    .await?;
    Ok(id.and_then(|s| s.parse().ok()))
}

pub async fn is_available(db: &SqlitePool, replica_id: Uuid) -> Result<bool, StorageError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_replicas
         WHERE id = ? AND state = 'ready'
           AND current_load < concurrency_limit",
    )
    .bind(replica_id.to_string())
    .fetch_one(db)
    .await?;
    Ok(count > 0)
}

pub async fn increment_load(db: &SqlitePool, replica_id: Uuid) -> Result<(), StorageError> {
    sqlx::query("UPDATE agent_replicas SET current_load = current_load + 1 WHERE id = ?")
        .bind(replica_id.to_string())
        .execute(db)
        .await?;
    Ok(())
}

pub async fn decrement_load(db: &SqlitePool, replica_id: Uuid) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE agent_replicas
         SET current_load = CASE WHEN current_load > 0 THEN current_load - 1 ELSE 0 END
         WHERE id = ?",
    )
    .bind(replica_id.to_string())
    .execute(db)
    .await?;
    Ok(())
}

pub async fn get_current_load(db: &SqlitePool, replica_id: Uuid) -> Result<i64, StorageError> {
    let load: i64 = sqlx::query_scalar(
        "SELECT current_load FROM agent_replicas WHERE id = ?",
    )
    .bind(replica_id.to_string())
    .fetch_optional(db)
    .await?
    .unwrap_or(0);
    Ok(load)
}

pub async fn get_backend_instance_id(
    db: &SqlitePool,
    replica_id: Uuid,
) -> Result<Option<String>, StorageError> {
    let id: Option<String> = sqlx::query_scalar(
        "SELECT backend_instance_id FROM agent_replicas WHERE id = ?",
    )
    .bind(replica_id.to_string())
    .fetch_optional(db)
    .await?;
    Ok(id)
}

pub async fn update_backend_instance_id(
    db: &SqlitePool,
    replica_id: Uuid,
    container_id: &str,
) -> Result<(), StorageError> {
    sqlx::query("UPDATE agent_replicas SET backend_instance_id = ? WHERE id = ?")
        .bind(container_id)
        .bind(replica_id.to_string())
        .execute(db)
        .await?;
    Ok(())
}
