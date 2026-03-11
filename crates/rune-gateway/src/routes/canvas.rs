use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::AppState;

pub async fn serve_canvas(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    if id.contains("..") || id.contains('/') || id.contains('\\') {
        return (StatusCode::BAD_REQUEST, "Invalid canvas ID").into_response();
    }

    let workspace = state.env.agent_workspace_dir.as_deref().unwrap_or(".");
    let path = std::path::PathBuf::from(workspace)
        .join(".rune")
        .join("canvas")
        .join(format!("{id}.html"));

    match tokio::fs::read_to_string(&path).await {
        Ok(html) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            html,
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "Canvas not found").into_response(),
    }
}
