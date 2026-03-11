use axum::{extract::State, http::StatusCode, Json};
use rune_storage::RuneStore;
use std::sync::Arc;

use crate::error::ControlPlaneError;
use crate::registry::{self, RegisterVersionInput};

pub async fn register_version(
    State(store): State<Arc<RuneStore>>,
    Json(req): Json<RegisterVersionInput>,
) -> Result<(StatusCode, Json<serde_json::Value>), ControlPlaneError> {
    let row = registry::register(&store, req).await?;
    Ok((StatusCode::CREATED, Json(serde_json::to_value(&row).unwrap())))
}

pub async fn list_versions(
    State(store): State<Arc<RuneStore>>,
) -> Result<Json<serde_json::Value>, ControlPlaneError> {
    let rows = registry::list(&store).await?;
    Ok(Json(serde_json::to_value(&rows).unwrap()))
}
