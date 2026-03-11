//! Threema adapter for rune-channels.
//!
//! Receives callbacks via HTTP webhook server. Sends via Threema Gateway REST API.

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

const THREEMA_GATEWAY: &str = "https://msgapi.threema.ch";
const MAX_MSG_LEN: usize = 3500;

/// Threema adapter.
pub struct ThreemaAdapter {
    api_identity: String,
    api_secret: Zeroizing<String>,
    webhook_port: u16,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl ThreemaAdapter {
    /// Create a new Threema adapter.
    pub fn new(api_identity: String, api_secret: String, webhook_port: u16) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            api_identity,
            api_secret: Zeroizing::new(api_secret),
            webhook_port,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
}

#[derive(Clone)]
struct ThreemaState {
    tx: mpsc::Sender<ChannelMessage>,
}

#[async_trait]
impl ChannelAdapter for ThreemaAdapter {
    fn name(&self) -> &str {
        "threema"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Threema
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let shutdown_rx = self.shutdown_rx.clone();

        let app = axum::Router::new()
            .route("/threema/webhook", axum::routing::post(threema_webhook_handler))
            .with_state(ThreemaState { tx: tx.clone() });

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

        info!(port = %self.webhook_port, "Threema webhook server started");
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let to = &user.platform_id;
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => {
                return Err(ChannelError::UnsupportedContent(
                    "Threema adapter only supports Text".to_string(),
                ))
            }
        };

        let chunks = split_message(&text, MAX_MSG_LEN);
        for chunk in chunks {
            let req_url = format!(
                "{}/send_simple?from={}&to={}&secret={}&text={}",
                THREEMA_GATEWAY,
                self.api_identity,
                to,
                self.api_secret.as_str(),
                urlencoding::encode(chunk)
            );

            let resp = self
                .client
                .get(&req_url)
                .send()
                .await
                .map_err(|e| ChannelError::Send(e.to_string()))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_body = resp.text().await.unwrap_or_default();
                return Err(ChannelError::Send(format!("{}: {}", status, err_body)));
            }

            let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
            if body.get("error").and_then(|v| v.as_str()).is_some() {
                return Err(ChannelError::Send(
                    body.get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                ));
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

async fn threema_webhook_handler(
    State(state): State<ThreemaState>,
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

    let from = value.get("from").and_then(|v| v.as_str());
    let to = value.get("to").and_then(|v| v.as_str());
    let message_id = value.get("messageId").and_then(|v| v.as_str());
    let text = value.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let nickname = value.get("nickname").and_then(|v| v.as_str());

    let from = match from {
        Some(f) => f,
        None => {
            return (
                axum::http::StatusCode::OK,
                axum::Json(serde_json::json!({ "ok": true })),
            )
                .into_response();
        }
    };

    let msg = ChannelMessage {
        channel: ChannelType::Threema,
        platform_message_id: message_id.map(String::from).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        sender: ChannelUser {
            platform_id: from.to_string(),
            display_name: nickname.unwrap_or(from).to_string(),
            rune_user: Some(from.to_string()),
        },
        content: ChannelContent::Text(text.to_string()),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: false,
        thread_id: to.map(String::from),
        metadata: HashMap::new(),
    };

    if state.tx.send(msg).await.is_err() {
        warn!("Threema channel closed, dropping message");
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
    fn test_threema_adapter_creation() {
        let adapter = ThreemaAdapter::new(
            "ECHOECHO".to_string(),
            "secret".to_string(),
            8768,
        );
        assert_eq!(adapter.name(), "threema");
        assert_eq!(adapter.channel_type(), ChannelType::Threema);
    }
}
