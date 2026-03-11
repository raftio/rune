use axum::{extract::{Path, State}, Json};
use uuid::Uuid;

use crate::error::GatewayError;
use crate::AppState;

pub async fn replica_health(
    State(state): State<AppState>,
    Path(replica_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let Some(ref backend) = state.backend else {
        return Ok(Json(serde_json::json!({
            "healthy": false,
            "message": "no runtime backend configured"
        })));
    };
    match backend.health(replica_id).await {
        Ok(h) => Ok(Json(serde_json::json!({
            "healthy": h.healthy,
            "message": h.message
        }))),
        Err(e) => Ok(Json(serde_json::json!({
            "healthy": false,
            "message": e.to_string()
        }))),
    }
}
