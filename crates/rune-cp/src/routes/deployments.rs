use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use rune_storage::RuneStore;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::ControlPlaneError;

#[derive(Debug, Deserialize)]
pub struct CreateDeploymentRequest {
    pub agent_version_id: Uuid,
    pub namespace: String,
    pub rollout_alias: String,
    #[serde(default = "default_one")]
    pub desired_replicas: i32,
    #[serde(default = "default_one")]
    pub min_replicas: i32,
    #[serde(default = "default_one")]
    pub max_replicas: i32,
    #[serde(default = "default_concurrency")]
    pub concurrency_limit: i32,
    pub config_ref: Option<String>,
    pub secret_ref: Option<String>,
    pub project_ref: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

fn default_one() -> i32 { 1 }
fn default_concurrency() -> i32 { 10 }

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DeploymentRow {
    pub id: String,
    pub agent_version_id: String,
    pub agent_name: String,
    pub namespace: String,
    pub rollout_alias: String,
    pub desired_replicas: i32,
    pub min_replicas: i32,
    pub max_replicas: i32,
    pub concurrency_limit: i32,
    pub routing_policy: String,
    pub resource_profile: String,
    pub config_ref: Option<String>,
    pub secret_ref: Option<String>,
    pub project_ref: Option<String>,
    pub env_json: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct ListDeploymentsQuery {
    pub project: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteDeploymentQuery {
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize)]
pub struct ScaleRequest {
    pub desired_replicas: i32,
}

pub async fn create_deployment(
    State(store): State<Arc<RuneStore>>,
    Json(req): Json<CreateDeploymentRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ControlPlaneError> {
    store.check_deployment_admission(req.agent_version_id).await?;

    if !store.agent_version_exists(req.agent_version_id).await? {
        return Err(ControlPlaneError::NotFound(format!(
            "agent_version {} not found", req.agent_version_id
        )));
    }

    let env_json = if req.env.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&req.env).unwrap_or_default())
    };

    let input = rune_storage::ops::deployments::CreateDeploymentInput {
        agent_version_id: req.agent_version_id,
        namespace: req.namespace,
        rollout_alias: req.rollout_alias,
        desired_replicas: req.desired_replicas,
        min_replicas: req.min_replicas,
        max_replicas: req.max_replicas,
        concurrency_limit: req.concurrency_limit,
        config_ref: req.config_ref,
        secret_ref: req.secret_ref,
        project_ref: req.project_ref,
        env_json,
    };

    let id = store.create_deployment(input).await?;

    let db = store.pool();
    let row: DeploymentRow = sqlx::query_as(
        "SELECT id, agent_version_id, agent_name, namespace, rollout_alias,
                desired_replicas, min_replicas, max_replicas, concurrency_limit,
                routing_policy, resource_profile,
                config_ref, secret_ref, project_ref, env_json, status,
                created_at, updated_at
         FROM agent_deployments WHERE id = ?",
    )
    .bind(id.to_string())
    .fetch_one(db)
    .await
    .map_err(ControlPlaneError::Database)?;

    Ok((StatusCode::CREATED, Json(serde_json::to_value(&row).unwrap())))
}

pub async fn list_deployments(
    State(store): State<Arc<RuneStore>>,
    Query(query): Query<ListDeploymentsQuery>,
) -> Result<Json<serde_json::Value>, ControlPlaneError> {
    let db = store.pool();
    let rows = if let Some(project) = &query.project {
        sqlx::query_as::<_, DeploymentRow>(
            "SELECT * FROM agent_deployments WHERE project_ref = ? ORDER BY created_at DESC",
        )
        .bind(project)
        .fetch_all(db)
        .await?
    } else {
        sqlx::query_as::<_, DeploymentRow>(
            "SELECT * FROM agent_deployments ORDER BY created_at DESC",
        )
        .fetch_all(db)
        .await?
    };

    Ok(Json(serde_json::json!({ "deployments": rows })))
}

pub async fn scale_deployment(
    State(store): State<Arc<RuneStore>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ScaleRequest>,
) -> Result<Json<serde_json::Value>, ControlPlaneError> {
    let affected = store.scale_deployment(id, req.desired_replicas).await?;

    if affected == 0 {
        return Err(ControlPlaneError::NotFound(format!("deployment {id} not found")));
    }

    Ok(Json(serde_json::json!({ "id": id, "desired_replicas": req.desired_replicas })))
}

pub async fn delete_deployment(
    State(store): State<Arc<RuneStore>>,
    Path(id): Path<Uuid>,
    Query(query): Query<DeleteDeploymentQuery>,
) -> Result<StatusCode, ControlPlaneError> {
    let affected = if query.force {
        store.delete_deployment_force(id).await?
    } else {
        store.delete_deployment(id).await?
    };

    if affected == 0 {
        return Err(ControlPlaneError::NotFound(format!("deployment {id} not found")));
    }

    Ok(StatusCode::NO_CONTENT)
}
