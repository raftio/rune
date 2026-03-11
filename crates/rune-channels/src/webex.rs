//! Cisco Webex adapter for rune-channels.
//!
//! Receives messages via webhook HTTP server. Sends via Webex REST API POST /messages.

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

const WEBEX_API: &str = "https://webexapis.com/v1";
const MAX_MSG_LEN: usize = 7439;

/// Cisco Webex adapter.
pub struct WebexAdapter {
    access_token: Zeroizing<String>,
    #[allow(dead_code)]
    webhook_url: Option<String>,
    webhook_port: u16,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl WebexAdapter {
    /// Create a new Webex adapter.
    pub fn new(
        access_token: String,
        webhook_url: Option<String>,
        webhook_port: u16,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            access_token: Zeroizing::new(access_token),
            webhook_url,
            webhook_port,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
}

#[derive(Clone)]
struct WebexState {
    tx: mpsc::Sender<ChannelMessage>,
}

#[async_trait]
impl ChannelAdapter for WebexAdapter {
    fn name(&self) -> &str {
        "webex"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Webex
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let shutdown_rx = self.shutdown_rx.clone();

        let app = axum::Router::new()
            .route("/webex/webhook", axum::routing::post(webex_webhook_handler))
            .with_state(WebexState { tx: tx.clone() });

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

        info!(port = %self.webhook_port, "Webex webhook server started");
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let room_id = &user.platform_id;
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            ChannelContent::Image { url, caption } => {
                format!("{} {}", url, caption.as_deref().unwrap_or("")).trim().to_string()
            }
            _ => {
                return Err(ChannelError::UnsupportedContent(
                    "Webex adapter only supports Text and Image".to_string(),
                ))
            }
        };

        let url = format!("{}/messages", WEBEX_API);
        let chunks = split_message(&text, MAX_MSG_LEN);

        for chunk in chunks {
            let body = serde_json::json!({
                "roomId": room_id,
                "markdown": chunk,
            });

            let resp = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.access_token.as_str()))
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

async fn webex_webhook_handler(
    State(state): State<WebexState>,
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

    let room_id = value.get("data").and_then(|d| d.get("roomId")).and_then(|v| v.as_str());
    let person_id = value.get("data").and_then(|d| d.get("personId")).and_then(|v| v.as_str());
    let msg_id = value.get("data").and_then(|d| d.get("id")).and_then(|v| v.as_str());
    let person_email = value.get("data").and_then(|d| d.get("personEmail")).and_then(|v| v.as_str());
    let text = value
        .get("data")
        .and_then(|d| d.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let (room_id, person_id, msg_id) = match (room_id, person_id, msg_id) {
        (Some(r), Some(p), Some(m)) => (r, p, m),
        _ => {
            return (
                axum::http::StatusCode::OK,
                axum::Json(serde_json::json!({ "ok": true })),
            )
                .into_response();
        }
    };

    let msg = ChannelMessage {
        channel: ChannelType::Webex,
        platform_message_id: msg_id.to_string(),
        sender: ChannelUser {
            platform_id: room_id.to_string(),
            display_name: person_email.unwrap_or(person_id).to_string(),
            rune_user: Some(person_id.to_string()),
        },
        content: ChannelContent::Text(text.to_string()),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: true,
        thread_id: Some(room_id.to_string()),
        metadata: HashMap::new(),
    };

    if state.tx.send(msg).await.is_err() {
        warn!("Webex channel closed, dropping message");
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
    fn test_webex_adapter_creation() {
        let adapter = WebexAdapter::new(
            "test-token".to_string(),
            Some("https://example.com/webhook".to_string()),
            8765,
        );
        assert_eq!(adapter.name(), "webex");
        assert_eq!(adapter.channel_type(), ChannelType::Webex);
    }
}
