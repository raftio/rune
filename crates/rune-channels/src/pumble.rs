//! Pumble adapter for rune-channels.
//!
//! Receives events via HTTP webhook server. Sends via Pumble API.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use axum::extract::State;
use axum::response::IntoResponse;
use chrono::Utc;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};
use zeroize::Zeroizing;

const PUMBLE_API: &str = "https://api.pumble.com";
const MAX_MSG_LEN: usize = 4000;

/// Pumble adapter.
pub struct PumbleAdapter {
    api_token: Zeroizing<String>,
    webhook_port: u16,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl PumbleAdapter {
    /// Create a new Pumble adapter.
    pub fn new(api_token: String, webhook_port: u16) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            api_token: Zeroizing::new(api_token),
            webhook_port,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
}

#[derive(Clone)]
struct PumbleState {
    tx: mpsc::Sender<ChannelMessage>,
}

#[async_trait]
impl ChannelAdapter for PumbleAdapter {
    fn name(&self) -> &str {
        "pumble"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Pumble
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let shutdown_rx = self.shutdown_rx.clone();

        let app = axum::Router::new()
            .route("/pumble/webhook", axum::routing::post(pumble_webhook_handler))
            .with_state(PumbleState { tx: tx.clone() });

        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", self.webhook_port))
            .await
            .map_err(|e| ChannelError::Config(format!("bind port {}: {}", self.webhook_port, e)))?;

        let mut shutdown_rx_clone = shutdown_rx.clone();
        let shutdown = async move {
            loop {
                if *shutdown_rx_clone.borrow() {
                    break;
                }
                if shutdown_rx_clone.changed().await.is_err() {
                    break;
                }
            }
        };

        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await;
        });

        info!(port = %self.webhook_port, "Pumble webhook server started");
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let channel_id = &user.platform_id;
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            ChannelContent::Image { url, caption } => {
                format!("{} {}", url, caption.as_deref().unwrap_or("")).trim().to_string()
            }
            _ => {
                return Err(ChannelError::UnsupportedContent(
                    "Pumble adapter only supports Text and Image".to_string(),
                ))
            }
        };

        let url = format!("{}/api/v1/messages", PUMBLE_API);
        let chunks = split_message(&text, MAX_MSG_LEN);

        for chunk in chunks {
            let body = serde_json::json!({
                "channelId": channel_id,
                "text": chunk,
            });

            let resp = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_token.as_str()))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| ChannelError::Send(e.to_string()))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_body = resp.text().await.unwrap_or_default();
                return Err(ChannelError::Send(format!("{}: {}", status, err_body)));
            }
        }

        Ok(())
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        ChannelStatus {
            connected: true,
            ..ChannelStatus::default()
        }
    }
}

async fn pumble_webhook_handler(
    State(state): State<PumbleState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let value: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({ "error": "invalid JSON" })),
            )
                .into_response();
        }
    };

    let event = value.get("event").and_then(|v| v.as_str());
    let msg_data = value.get("message").or_else(|| value.get("data"));
    let message_id = msg_data
        .and_then(|d| d.get("id"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let text = msg_data
        .and_then(|d| d.get("text").or_else(|| d.get("content")))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let user_id = msg_data
        .and_then(|d| d.get("userId").or_else(|| d.get("user_id")))
        .and_then(|v| v.as_str());
    let user_name = msg_data
        .and_then(|d| d.get("userName").or_else(|| d.get("user_name").or_else(|| d.get("displayName"))))
        .and_then(|v| v.as_str());
    let channel_id = msg_data
        .and_then(|d| d.get("channelId").or_else(|| d.get("channel_id")))
        .and_then(|v| v.as_str());

    let (user_id, channel_id) = match (user_id, channel_id) {
        (Some(u), Some(c)) => (u, c),
        (Some(u), None) => (u, u),
        (None, Some(c)) => (c, c),
        _ => {
            return (
                axum::http::StatusCode::OK,
                axum::Json(serde_json::json!({ "ok": true })),
            )
                .into_response();
        }
    };

    if event != Some("message") && event != Some("message.created") && event.is_some() {
        return (
            axum::http::StatusCode::OK,
            axum::Json(serde_json::json!({ "ok": true })),
        )
            .into_response();
    }

    let msg = ChannelMessage {
        channel: ChannelType::Pumble,
        platform_message_id: message_id,
        sender: ChannelUser {
            platform_id: channel_id.to_string(),
            display_name: user_name.unwrap_or(user_id).to_string(),
            rune_user: Some(user_id.to_string()),
        },
        content: ChannelContent::Text(text.to_string()),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: true,
        thread_id: Some(channel_id.to_string()),
        metadata: HashMap::new(),
    };

    if state.tx.send(msg).await.is_err() {
        warn!("Pumble channel closed, dropping message");
    }

    (
        axum::http::StatusCode::OK,
        axum::Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pumble_adapter_creation() {
        let adapter = PumbleAdapter::new("test-token".to_string(), 8767);
        assert_eq!(adapter.name(), "pumble");
        assert_eq!(adapter.channel_type(), ChannelType::Pumble);
    }
}
