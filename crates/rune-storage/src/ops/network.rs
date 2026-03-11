use sqlx::SqlitePool;

use crate::error::StorageError;

pub async fn get_agent_networks(
    db: &SqlitePool,
    agent_name: &str,
) -> Result<Vec<String>, StorageError> {
    let networks: Option<String> = sqlx::query_scalar(
        "SELECT d.networks FROM agent_deployments d
         JOIN agent_versions v ON v.id = d.agent_version_id
         WHERE v.agent_name = ? AND d.status = 'active'
         LIMIT 1",
    )
    .bind(agent_name)
    .fetch_optional(db)
    .await?;

    let json = networks.unwrap_or_else(|| r#"["bridge"]"#.into());
    Ok(serde_json::from_str::<Vec<String>>(&json).unwrap_or_else(|_| vec!["bridge".into()]))
}

pub async fn get_direct_replica_endpoint(
    db: &SqlitePool,
    agent_name: &str,
) -> Result<Option<String>, StorageError> {
    let endpoint: Option<Option<String>> = sqlx::query_scalar(
        "SELECT r.endpoint FROM agent_replicas r
         JOIN agent_deployments d ON r.deployment_id = d.id
         JOIN agent_versions v ON v.id = d.agent_version_id
         WHERE v.agent_name = ? AND d.status = 'active'
           AND r.state = 'ready'
           AND r.current_load < r.concurrency_limit
           AND r.endpoint IS NOT NULL
         ORDER BY r.current_load ASC
         LIMIT 1",
    )
    .bind(agent_name)
    .fetch_optional(db)
    .await?;

    Ok(endpoint.flatten())
}
