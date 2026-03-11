use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde_json::json;

use crate::AppState;

/// POST /channels/webhook — receive incoming webhook messages.
///
/// Requires `RUNE_WEBHOOK_SECRET` to be set; rejects all requests otherwise.
pub async fn webhook_inbound(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let bridge = match &state.bridge {
        Some(b) => b,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "Channel bridge not configured" })),
            )
                .into_response();
        }
    };

    // Enforce webhook secret — never allow unauthenticated webhook ingestion
    let secret = state.env.webhook_secret.clone().unwrap_or_default();
    if secret.is_empty() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Webhook secret not configured" })),
        )
            .into_response();
    }

    let signature = headers
        .get("X-Webhook-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !rune_channels::webhook::verify_signature(&secret, &body, signature) {
        // Constant-time verification already done inside verify_signature;
        // always sleep briefly to avoid timing-based probing
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Invalid signature" })),
        )
            .into_response();
    }

    let json_body: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Invalid JSON" })),
            )
                .into_response();
        }
    };

    let message = json_body
        .get("message")
        .or_else(|| json_body.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if message.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Empty message" })),
        )
            .into_response();
    }

    let agent_name = match json_body.get("agent").and_then(|v| v.as_str()) {
        Some(name) if !name.is_empty() && name.len() <= 128 => name,
        Some(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Invalid 'agent' field" })),
            )
                .into_response();
        }
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing 'agent' field" })),
            )
                .into_response();
        }
    };

    // Constant-time-ish lookup: always perform the DB query regardless of
    // whether the name looks valid, then return a generic error.
    let agent_id = match bridge.find_agent_by_name(agent_name).await {
        Ok(Some(id)) => id,
        Ok(None) | Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Agent not found" })),
            )
                .into_response();
        }
    };

    match bridge.send_message(agent_id, message).await {
        Ok(response) => (StatusCode::OK, Json(json!({ "response": response }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

/// GET /channels/status — return health status of all running adapters
pub async fn adapter_status(State(state): State<AppState>) -> impl IntoResponse {
    let statuses = match &state.bridge {
        Some(b) => b.adapter_statuses(),
        None => vec![],
    };

    let entries: Vec<serde_json::Value> = statuses
        .iter()
        .map(|(name, status)| {
            json!({
                "name": name,
                "connected": status.connected,
                "messages_received": status.messages_received,
                "messages_sent": status.messages_sent,
                "last_error": status.last_error,
            })
        })
        .collect();

    Json(json!({ "adapters": entries }))
}
