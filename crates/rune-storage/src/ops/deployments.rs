use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::{Deployment, DeploymentStatus};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CreateDeploymentInput {
    pub agent_version_id: Uuid,
    pub namespace: String,
    pub rollout_alias: String,
    pub desired_replicas: i32,
    pub min_replicas: i32,
    pub max_replicas: i32,
    pub concurrency_limit: i32,
    pub config_ref: Option<String>,
    pub secret_ref: Option<String>,
    pub project_ref: Option<String>,
    pub env_json: Option<String>,
}

#[derive(sqlx::FromRow)]
struct DeploymentRow {
    id: String,
    agent_version_id: String,
    namespace: String,
    rollout_alias: String,
    desired_replicas: i32,
    min_replicas: i32,
    max_replicas: i32,
    concurrency_limit: i32,
    config_ref: Option<String>,
    secret_ref: Option<String>,
    env_json: Option<String>,
    status: String,
    created_at: String,
    updated_at: String,
}

fn row_to_deployment(r: DeploymentRow) -> Option<Deployment> {
    let id = r.id.parse().ok()?;
    let agent_version_id = r.agent_version_id.parse().ok()?;
    let created_at = chrono::DateTime::parse_from_rfc3339(&r.created_at)
        .ok()?
        .with_timezone(&chrono::Utc);
    let updated_at = chrono::DateTime::parse_from_rfc3339(&r.updated_at)
        .ok()?
        .with_timezone(&chrono::Utc);
    Some(Deployment {
        id,
        agent_version_id,
        namespace: r.namespace,
        rollout_alias: r.rollout_alias,
        desired_replicas: r.desired_replicas,
        min_replicas: r.min_replicas,
        max_replicas: r.max_replicas,
        concurrency_limit: r.concurrency_limit,
        config_ref: r.config_ref,
        secret_ref: r.secret_ref,
        env_json: r.env_json,
        status: match r.status.as_str() {
            "active" => DeploymentStatus::Active,
            "draining" => DeploymentStatus::Draining,
            "stopped" => DeploymentStatus::Stopped,
            _ => DeploymentStatus::Pending,
        },
        created_at,
        updated_at,
    })
}

pub async fn get_schedulable(db: &SqlitePool) -> Result<Vec<Deployment>, StorageError> {
    let rows = sqlx::query_as::<_, DeploymentRow>(
        "SELECT id, agent_version_id, namespace, rollout_alias,
                desired_replicas, min_replicas, max_replicas, concurrency_limit,
                config_ref, secret_ref, env_json, status, created_at, updated_at
         FROM agent_deployments WHERE status IN ('pending', 'active')",
    )
    .fetch_all(db)
    .await?;

    Ok(rows.into_iter().filter_map(row_to_deployment).collect())
}

/// Create or upsert a deployment. Used by control plane via Raft so it is applied
/// on the same node that has the agent_version, avoiding FK violations in cluster mode.
pub async fn create(
    db: &SqlitePool,
    input: &CreateDeploymentInput,
) -> Result<Uuid, StorageError> {
    let agent_name: String = sqlx::query_scalar(
        "SELECT agent_name FROM agent_versions WHERE id = ?",
    )
    .bind(input.agent_version_id.to_string())
    .fetch_one(db)
    .await
    .map_err(|e| {
        StorageError::NotFound(format!(
            "agent_version {} not found (deployment must be routed through Raft): {}",
            input.agent_version_id, e
        ))
    })?;

    let id = Uuid::new_v4();
    let row: (String,) = sqlx::query_as(
        "INSERT INTO agent_deployments
            (id, agent_version_id, agent_name, namespace, rollout_alias,
             desired_replicas, min_replicas, max_replicas, concurrency_limit,
             routing_policy, resource_profile,
             config_ref, secret_ref, project_ref, env_json, status)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, '{}', '{}', ?, ?, ?, ?, 'pending')
         ON CONFLICT (namespace, rollout_alias, agent_name) DO UPDATE SET
             agent_version_id  = excluded.agent_version_id,
             desired_replicas  = excluded.desired_replicas,
             min_replicas      = excluded.min_replicas,
             max_replicas      = excluded.max_replicas,
             concurrency_limit = excluded.concurrency_limit,
             config_ref        = excluded.config_ref,
             secret_ref        = excluded.secret_ref,
             project_ref       = excluded.project_ref,
             env_json          = excluded.env_json,
             status            = 'pending',
             updated_at        = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         RETURNING id",
    )
    .bind(id.to_string())
    .bind(input.agent_version_id.to_string())
    .bind(&agent_name)
    .bind(&input.namespace)
    .bind(&input.rollout_alias)
    .bind(input.desired_replicas)
    .bind(input.min_replicas)
    .bind(input.max_replicas)
    .bind(input.concurrency_limit)
    .bind(&input.config_ref)
    .bind(&input.secret_ref)
    .bind(&input.project_ref)
    .bind(&input.env_json)
    .fetch_one(db)
    .await?;
    row.0
        .parse()
        .map_err(|_| StorageError::Serialization("invalid deployment id".into()))
}

pub async fn set_active(db: &SqlitePool, id: Uuid) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE agent_deployments SET status = 'active', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
    )
    .bind(id.to_string())
    .execute(db)
    .await?;
    Ok(())
}

/// Get agent_name and env_json for a deployment by id. Used by channel bridge
/// to resolve agent package path when AGENT_PACKAGES_DIR/RUNE_CHANNEL_AGENT
/// are not set in the gateway process env.
pub async fn get_agent_and_env_for_deployment(
    db: &SqlitePool,
    deployment_id: Uuid,
) -> Result<Option<(String, Option<String>)>, StorageError> {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT agent_name, env_json FROM agent_deployments
         WHERE id = ? AND status = 'active'",
    )
    .bind(deployment_id.to_string())
    .fetch_optional(db)
    .await?;
    Ok(row)
}

pub async fn resolve_for_agent(
    db: &SqlitePool,
    agent_name: &str,
) -> Result<Option<Uuid>, StorageError> {
    let id: Option<String> = sqlx::query_scalar(
        "SELECT d.id FROM agent_deployments d
         JOIN agent_versions v ON v.id = d.agent_version_id
         WHERE v.agent_name = ? AND d.status = 'active'
         LIMIT 1",
    )
    .bind(agent_name)
    .fetch_optional(db)
    .await?;
    Ok(id.and_then(|s| s.parse().ok()))
}

pub async fn scale(
    db: &SqlitePool,
    id: Uuid,
    desired_replicas: i32,
) -> Result<u64, StorageError> {
    let affected = sqlx::query(
        "UPDATE agent_deployments
         SET desired_replicas = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?",
    )
    .bind(desired_replicas)
    .bind(id.to_string())
    .execute(db)
    .await?
    .rows_affected();
    Ok(affected)
}

pub async fn delete(db: &SqlitePool, id: Uuid) -> Result<u64, StorageError> {
    let id_str = id.to_string();

    // Delete in FK-safe order: children before parents.
    // 1. tool_invocations → agent_requests
    sqlx::query(
        "DELETE FROM tool_invocations WHERE request_id IN
             (SELECT id FROM agent_requests WHERE deployment_id = ?)",
    )
    .bind(&id_str)
    .execute(db)
    .await?;

    // 2. a2a_tasks → agent_requests / agent_sessions
    sqlx::query(
        "DELETE FROM a2a_tasks WHERE request_id IN
             (SELECT id FROM agent_requests WHERE deployment_id = ?)",
    )
    .bind(&id_str)
    .execute(db)
    .await?;

    // 3. agent_requests → agent_deployments
    sqlx::query("DELETE FROM agent_requests WHERE deployment_id = ?")
        .bind(&id_str)
        .execute(db)
        .await?;

    // 4. session_checkpoints → agent_sessions
    sqlx::query(
        "DELETE FROM session_checkpoints WHERE session_id IN
             (SELECT id FROM agent_sessions WHERE deployment_id = ?)",
    )
    .bind(&id_str)
    .execute(db)
    .await?;

    // 5. session_messages → agent_sessions
    sqlx::query(
        "DELETE FROM session_messages WHERE session_id IN
             (SELECT id FROM agent_sessions WHERE deployment_id = ?)",
    )
    .bind(&id_str)
    .execute(db)
    .await?;

    // 6. agent_sessions → agent_deployments (also drops assigned_replica_id refs)
    sqlx::query("DELETE FROM agent_sessions WHERE deployment_id = ?")
        .bind(&id_str)
        .execute(db)
        .await?;

    // 7. agent_replicas → agent_deployments (safe now that sessions are gone)
    sqlx::query("DELETE FROM agent_replicas WHERE deployment_id = ?")
        .bind(&id_str)
        .execute(db)
        .await?;

    // 8. agent_deployments (final)
    let affected = sqlx::query("DELETE FROM agent_deployments WHERE id = ?")
        .bind(&id_str)
        .execute(db)
        .await?
        .rows_affected();

    Ok(affected)
}

/// Force-delete deployment by disabling foreign keys. Leaves orphan rows in
/// dependent tables; use only when normal cascade delete fails.
pub async fn delete_force(db: &SqlitePool, id: Uuid) -> Result<u64, StorageError> {
    let id_str = id.to_string();
    let mut conn = sqlx::Acquire::acquire(db).await?;

    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *conn)
        .await?;

    let affected = sqlx::query("DELETE FROM agent_deployments WHERE id = ?")
        .bind(&id_str)
        .execute(&mut *conn)
        .await?
        .rows_affected();

    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await?;

    Ok(affected)
}

pub async fn list_deployed_agent_names(db: &SqlitePool) -> Result<Vec<String>, StorageError> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT v.agent_name FROM agent_versions v
         JOIN agent_deployments d ON v.id = d.agent_version_id
         WHERE d.status = 'active'",
    )
    .fetch_all(db)
    .await?;
    Ok(names)
}

pub async fn verify_agent_deployed(
    db: &SqlitePool,
    agent_name: &str,
) -> Result<bool, StorageError> {
    let exists: Option<String> = sqlx::query_scalar(
        "SELECT d.id FROM agent_deployments d
         JOIN agent_versions v ON v.id = d.agent_version_id
         WHERE v.agent_name = ? AND d.status = 'active'
         LIMIT 1",
    )
    .bind(agent_name)
    .fetch_optional(db)
    .await?;
    Ok(exists.is_some())
}
