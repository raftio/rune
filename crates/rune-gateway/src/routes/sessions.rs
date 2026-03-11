use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::error::GatewayError;
use rune_runtime::SessionManager;
use rune_storage::RuntimeStore;

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub agent_name: String,
    pub tenant_id: Option<String>,
    pub routing_key: Option<String>,
}

pub async fn create_session(
    State(state): State<crate::AppState>,
    Path(_agent_name): Path<String>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), GatewayError> {
    let deployment_id: Uuid = state.store
        .resolve_deployment_for_agent(&req.agent_name)
        .await
        .map_err(rune_runtime::RuntimeError::Storage)?
        .ok_or_else(|| {
            GatewayError::NotFound(format!("no active deployment for agent '{}'", req.agent_name))
        })?;

    let sessions = SessionManager::new(state.store.clone());
    let session_id = sessions.create(deployment_id, req.tenant_id, req.routing_key).await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "session_id": session_id,
            "deployment_id": deployment_id,
            "status": "active"
        })),
    ))
}

pub async fn get_session(
    State(state): State<crate::AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let info = state.store
        .get_session_info(session_id)
        .await
        .map_err(rune_runtime::RuntimeError::RuntimeStore)?;

    match info {
        None => Err(GatewayError::NotFound(format!("session {session_id} not found"))),
        Some(v) => Ok(Json(v)),
    }
}
