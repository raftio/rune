//! Flock adapter for rune-channels.
//!
//! Receives events via HTTP webhook server. Sends via Flock chat.sendMessage API.

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

const FLOCK_API: &str = "https://api.flock.com";
const MAX_MSG_LEN: usize = 4000;

/// Flock adapter.
pub struct FlockAdapter {
    bot_token: Zeroizing<String>,
    webhook_port: u16,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl FlockAdapter {
    /// Create a new Flock adapter.
    pub fn new(bot_token: String, webhook_port: u16) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            bot_token: Zeroizing::new(bot_token),
            webhook_port,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
}

#[derive(Clone)]
struct FlockState {
    tx: mpsc::Sender<ChannelMessage>,
}

#[async_trait]
impl ChannelAdapter for FlockAdapter {
    fn name(&self) -> &str {
        "flock"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Flock
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let shutdown_rx = self.shutdown_rx.clone();

        let app = axum::Router::new()
            .route("/flock/webhook", axum::routing::post(flock_webhook_handler))
            .with_state(FlockState { tx: tx.clone() });

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

        info!(port = %self.webhook_port, "Flock webhook server started");
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let to = &user.platform_id;
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            ChannelContent::Image { url, caption } => {
                format!("{} {}", url, caption.as_deref().unwrap_or("")).trim().to_string()
            }
            _ => {
                return Err(ChannelError::UnsupportedContent(
                    "Flock adapter only supports Text and Image".to_string(),
                ))
            }
        };

        let url = format!("{}/v1/chat.sendMessage", FLOCK_API);
        let chunks = split_message(&text, MAX_MSG_LEN);

        for chunk in chunks {
            let body = serde_json::json!({
                "to": to,
                "text": chunk,
            });

            let resp = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.bot_token.as_str()))
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

async fn flock_webhook_handler(
    State(state): State<FlockState>,
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

    let name = value.get("name").and_then(|v| v.as_str());
    let user_id = value.get("userId").and_then(|v| v.as_str());
    let chat = value.get("chat").and_then(|c| c.as_str());
    let text = value.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let msg_uid = value
        .get("messageUid")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let (user_id, chat_id) = match (user_id, chat) {
        (Some(u), Some(c)) => (u, c),
        _ => {
            return (
                axum::http::StatusCode::OK,
                axum::Json(serde_json::json!({ "ok": true })),
            )
                .into_response();
        }
    };

    let msg = ChannelMessage {
        channel: ChannelType::Flock,
        platform_message_id: msg_uid,
        sender: ChannelUser {
            platform_id: chat_id.to_string(),
            display_name: name.unwrap_or(user_id).to_string(),
            rune_user: Some(user_id.to_string()),
        },
        content: ChannelContent::Text(text.to_string()),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: chat_id.starts_with("g:"),
        thread_id: Some(chat_id.to_string()),
        metadata: HashMap::new(),
    };

    if state.tx.send(msg).await.is_err() {
        warn!("Flock channel closed, dropping message");
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
    fn test_flock_adapter_creation() {
        let adapter = FlockAdapter::new("test-token".to_string(), 8766);
        assert_eq!(adapter.name(), "flock");
        assert_eq!(adapter.channel_type(), ChannelType::Flock);
    }
}
