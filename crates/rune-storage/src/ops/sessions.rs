use sqlx::SqlitePool;
use uuid::Uuid;

use rune_store_runtime::{CheckpointBlob, SessionMessage};

use crate::error::StorageError;
use crate::object_store::ObjectStoreAdapter;

pub async fn create(
    db: &SqlitePool,
    deployment_id: Uuid,
    tenant_id: Option<&str>,
    routing_key: Option<&str>,
) -> Result<Uuid, StorageError> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_sessions (id, deployment_id, tenant_id, routing_key, status)
         VALUES (?, ?, ?, ?, 'active')",
    )
    .bind(id.to_string())
    .bind(deployment_id.to_string())
    .bind(tenant_id)
    .bind(routing_key)
    .execute(db)
    .await?;
    Ok(id)
}

pub async fn load_messages(
    db: &SqlitePool,
    session_id: Uuid,
) -> Result<Vec<SessionMessage>, StorageError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: String,
        session_id: String,
        role: String,
        content: String,
        step: i64,
        created_at: String,
    }

    let rows = sqlx::query_as::<_, Row>(
        "SELECT id, session_id, role, content, step, created_at
         FROM session_messages WHERE session_id = ? ORDER BY created_at ASC",
    )
    .bind(session_id.to_string())
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| SessionMessage {
            id: r.id,
            session_id: r.session_id,
            role: r.role,
            content: serde_json::from_str(&r.content)
                .unwrap_or(serde_json::Value::String(r.content.clone())),
            step: r.step,
            created_at: r.created_at,
        })
        .collect())
}

pub async fn append_message(
    db: &SqlitePool,
    session_id: Uuid,
    role: &str,
    content: &serde_json::Value,
    step: i64,
) -> Result<(), StorageError> {
    let id = Uuid::new_v4().to_string();
    let content_str = serde_json::to_string(content)
        .map_err(|e| StorageError::Serialization(e.to_string()))?;
    sqlx::query(
        "INSERT INTO session_messages (id, session_id, role, content, step)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(session_id.to_string())
    .bind(role)
    .bind(&content_str)
    .bind(step)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn close(db: &SqlitePool, session_id: Uuid) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE agent_sessions
         SET status = 'closed', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?",
    )
    .bind(session_id.to_string())
    .execute(db)
    .await?;
    Ok(())
}

pub async fn get_deployment_id(
    db: &SqlitePool,
    session_id: Uuid,
) -> Result<Option<Uuid>, StorageError> {
    let row: Option<String> = sqlx::query_scalar(
        "SELECT deployment_id FROM agent_sessions WHERE id = ? AND status = 'active'",
    )
    .bind(session_id.to_string())
    .fetch_optional(db)
    .await?;
    Ok(row.and_then(|s| s.parse().ok()))
}

pub async fn get_assigned_replica(
    db: &SqlitePool,
    session_id: Uuid,
) -> Result<Option<Uuid>, StorageError> {
    let id: Option<Option<String>> = sqlx::query_scalar(
        "SELECT assigned_replica_id FROM agent_sessions
         WHERE id = ? AND status = 'active'",
    )
    .bind(session_id.to_string())
    .fetch_optional(db)
    .await?;
    Ok(id.flatten().and_then(|s| s.parse().ok()))
}

pub async fn assign_replica(
    db: &SqlitePool,
    session_id: Uuid,
    replica_id: Uuid,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE agent_sessions
         SET assigned_replica_id = ?,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?",
    )
    .bind(replica_id.to_string())
    .bind(session_id.to_string())
    .execute(db)
    .await?;
    Ok(())
}

pub async fn checkpoint(
    db: &SqlitePool,
    session_id: Uuid,
    messages: &[SessionMessage],
    step: i64,
) -> Result<(), StorageError> {
    let blob = CheckpointBlob {
        messages: messages.to_vec(),
        step,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let data = serde_json::to_string(&blob)
        .map_err(|e| StorageError::Serialization(e.to_string()))?;
    let data_bytes = data.as_bytes();

    let (data_to_store, storage_ref): (String, Option<String>) =
        if let Some(ref store) = ObjectStoreAdapter::from_env() {
            if store.should_store_in_object_store(data_bytes.len()) {
                let path_ref = store.put_checkpoint(session_id, data_bytes).await?;
                ("".to_string(), Some(path_ref))
            } else {
                (data, None)
            }
        } else {
            (data, None)
        };

    sqlx::query(
        "INSERT INTO session_checkpoints (session_id, data, step, created_at, storage_ref)
         VALUES (?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?)
         ON CONFLICT(session_id) DO UPDATE SET data = excluded.data, step = excluded.step, created_at = excluded.created_at, storage_ref = excluded.storage_ref",
    )
    .bind(session_id.to_string())
    .bind(&data_to_store)
    .bind(step)
    .bind(&storage_ref)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn restore(
    db: &SqlitePool,
    session_id: Uuid,
) -> Result<Option<(Vec<SessionMessage>, i64)>, StorageError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        data: Option<String>,
        step: i64,
        storage_ref: Option<String>,
    }

    let row: Option<Row> = sqlx::query_as(
        "SELECT data, step, storage_ref FROM session_checkpoints WHERE session_id = ?",
    )
    .bind(session_id.to_string())
    .fetch_optional(db)
    .await?;

    match row {
        Some(r) => {
            let data = if let Some(ref path_ref) = r.storage_ref {
                let store = ObjectStoreAdapter::from_env().ok_or_else(|| {
                    StorageError::ObjectStore(
                        "checkpoint in object storage but RUNE_OBJECT_STORE_URL not configured"
                            .into(),
                    )
                })?;
                let bytes = store.get_checkpoint(path_ref).await?;
                String::from_utf8(bytes).map_err(|e| {
                    StorageError::Serialization(format!("checkpoint decode: {e}"))
                })?
            } else {
                r.data.unwrap_or_default()
            };
            if data.is_empty() {
                return Ok(None);
            }
            let blob: CheckpointBlob = serde_json::from_str(&data)
                .map_err(|e| StorageError::Serialization(format!("invalid checkpoint: {e}")))?;
            Ok(Some((blob.messages, r.step)))
        }
        None => Ok(None),
    }
}

pub async fn apply_restore(
    db: &SqlitePool,
    session_id: Uuid,
    messages: &[SessionMessage],
) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM session_messages WHERE session_id = ?")
        .bind(session_id.to_string())
        .execute(db)
        .await?;

    for (i, msg) in messages.iter().enumerate() {
        let content_str = serde_json::to_string(&msg.content)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        sqlx::query(
            "INSERT INTO session_messages (id, session_id, role, content, step)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&msg.id)
        .bind(session_id.to_string())
        .bind(&msg.role)
        .bind(&content_str)
        .bind(i as i64)
        .execute(db)
        .await?;
    }
    Ok(())
}

pub async fn get_session_info(
    db: &SqlitePool,
    session_id: Uuid,
) -> Result<Option<serde_json::Value>, StorageError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: String,
        deployment_id: String,
        tenant_id: Option<String>,
        routing_key: Option<String>,
        status: String,
        created_at: String,
        updated_at: String,
    }

    let row = sqlx::query_as::<_, Row>(
        "SELECT id, deployment_id, tenant_id, routing_key, status, created_at, updated_at
         FROM agent_sessions WHERE id = ?",
    )
    .bind(session_id.to_string())
    .fetch_optional(db)
    .await?;

    Ok(row.map(|r| {
        serde_json::json!({
            "id": r.id,
            "deployment_id": r.deployment_id,
            "tenant_id": r.tenant_id,
            "routing_key": r.routing_key,
            "status": r.status,
            "created_at": r.created_at,
            "updated_at": r.updated_at,
        })
    }))
}
