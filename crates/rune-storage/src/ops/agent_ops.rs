use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::StorageError;

pub async fn list_agents(db: &SqlitePool) -> Result<Vec<AgentRow>, StorageError> {
    let rows: Vec<AgentRow> = sqlx::query_as(
        "SELECT d.id, v.agent_name, d.status, d.project_ref, d.desired_replicas
         FROM agent_deployments d
         JOIN agent_versions v ON v.id = d.agent_version_id
         ORDER BY d.created_at DESC
         LIMIT 50",
    )
    .fetch_all(db)
    .await?;
    Ok(rows)
}

#[derive(sqlx::FromRow)]
pub struct AgentRow {
    pub id: String,
    pub agent_name: String,
    pub status: String,
    pub project_ref: Option<String>,
    pub desired_replicas: i64,
}

pub async fn find_agents(
    db: &SqlitePool,
    query: &str,
) -> Result<Vec<AgentSearchRow>, StorageError> {
    let pattern = format!("%{query}%");
    let rows: Vec<AgentSearchRow> = sqlx::query_as(
        "SELECT d.id, v.agent_name, d.status
         FROM agent_deployments d
         JOIN agent_versions v ON v.id = d.agent_version_id
         WHERE v.agent_name LIKE ? OR d.project_ref LIKE ?
         ORDER BY d.created_at DESC
         LIMIT 20",
    )
    .bind(&pattern)
    .bind(&pattern)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

#[derive(sqlx::FromRow)]
pub struct AgentSearchRow {
    pub id: String,
    pub agent_name: String,
    pub status: String,
}

pub async fn spawn_agent(
    db: &SqlitePool,
    agent_name: &str,
) -> Result<(String, String), StorageError> {
    let version_id = Uuid::new_v4().to_string();
    let deployment_id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO agent_versions
            (id, agent_name, version, image_ref, image_digest, spec_sha256, runtime_class, status)
         VALUES (?, ?, '0.1.0', 'local', '', '', 'builtin', 'active')",
    )
    .bind(&version_id)
    .bind(agent_name)
    .execute(db)
    .await?;

    sqlx::query(
        "INSERT INTO agent_deployments
            (id, agent_version_id, namespace, rollout_alias, status, desired_replicas, project_ref)
         VALUES (?, ?, 'default', 'stable', 'active', 1, ?)",
    )
    .bind(&deployment_id)
    .bind(&version_id)
    .bind(agent_name)
    .execute(db)
    .await?;

    Ok((version_id, deployment_id))
}

pub async fn kill_agent(db: &SqlitePool, agent_id: &str) -> Result<u64, StorageError> {
    let result = sqlx::query(
        "UPDATE agent_deployments SET status = 'terminated', desired_replicas = 0 WHERE id = ?",
    )
    .bind(agent_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}
