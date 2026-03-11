//! LinkedIn adapter for rune-channels.
//!
//! Receives webhook events and sends via LinkedIn Messaging API.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use axum::{
    extract::State,
    routing::post,
    Json, Router,
};
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

const LINKEDIN_API: &str = "https://api.linkedin.com/rest";
const MAX_MSG_LEN: usize = 8000;

/// LinkedIn messaging adapter.
pub struct LinkedInAdapter {
    access_token: Zeroizing<String>,
    webhook_port: u16,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

#[derive(Clone)]
struct LinkedInState {
    tx: mpsc::Sender<ChannelMessage>,
}

impl LinkedInAdapter {
    /// Create a new LinkedIn adapter.
    pub fn new(access_token: String, webhook_port: u16) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            access_token: Zeroizing::new(access_token),
            webhook_port,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
}

#[async_trait]
impl ChannelAdapter for LinkedInAdapter {
    fn name(&self) -> &str {
        "linkedin"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::LinkedIn
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let shutdown_rx = self.shutdown_rx.clone();
        let webhook_port = self.webhook_port;

        let app = Router::new()
            .route("/linkedin/webhook", post(linkedin_webhook_handler))
            .route("/linkedin", post(linkedin_webhook_handler))
            .with_state(LinkedInState { tx: tx.clone() });

        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", webhook_port))
            .await
            .map_err(|e| ChannelError::Config(format!("bind to port {}: {}", webhook_port, e)))?;

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
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await
            {
                warn!(error = %e, "LinkedIn webhook server stopped");
            }
        });

        info!(port = %webhook_port, "LinkedIn adapter started");
        let stream = ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "LinkedIn adapter only supports Text content".to_string(),
            )),
        };

        // user.platform_id is the conversation URN or participant URN
        let conversation_urn = &user.platform_id;

        let url = format!(
            "{}/messages?q=conversationUrn",
            LINKEDIN_API
        );

        let chunks = split_message(&text, MAX_MSG_LEN);
        for chunk in chunks {
            let body = serde_json::json!({
                "event": {
                    "subtype": "MESSAGE",
                    "actor": "urn:li:person:placeholder",
                    "created": Utc::now().timestamp_millis()
                },
                "message": {
                    "subtype": "MESSAGE",
                    "conversationUrn": conversation_urn,
                    "body": chunk
                }
            });

            let resp = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.access_token.as_str()))
                .header("Content-Type", "application/json")
                .header("LinkedIn-Version", "202401")
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_body = resp.text().await.unwrap_or_default();
                return Err(ChannelError::Send(format!(
                    "HTTP {}: {}",
                    status,
                    err_body.lines().next().unwrap_or(&err_body)
                )));
            }
        }

        Ok(())
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        ChannelStatus::default()
    }
}

async fn linkedin_webhook_handler(
    State(state): State<LinkedInState>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let entity_urn = body
        .get("entityUrn")
        .or_else(|| body.get("entity"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let actor = body.get("actor").and_then(|a| a.as_str()).unwrap_or("unknown");
    let message = body
        .get("message")
        .and_then(|m| m.as_object())
        .and_then(|o| o.get("body"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let event_subtype = body
        .get("event")
        .and_then(|e| e.as_object())
        .and_then(|o| o.get("subtype"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if event_subtype != "MESSAGE" || message.is_empty() {
        return Json(serde_json::json!({ "ok": true }));
    }

    let msg = ChannelMessage {
        channel: ChannelType::LinkedIn,
        platform_message_id: uuid::Uuid::new_v4().to_string(),
        sender: ChannelUser {
            platform_id: entity_urn.clone(),
            display_name: actor.to_string(),
            rune_user: None,
        },
        content: ChannelContent::Text(message.to_string()),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: false,
        thread_id: None,
        metadata: HashMap::new(),
    };

    if state.tx.send(msg).await.is_err() {
        warn!("LinkedIn channel closed, dropping message");
    }

    Json(serde_json::json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linkedin_adapter_creation() {
        let adapter = LinkedInAdapter::new("access_token".to_string(), 19003);
        assert_eq!(adapter.name(), "linkedin");
        assert_eq!(adapter.channel_type(), ChannelType::LinkedIn);
    }
}
