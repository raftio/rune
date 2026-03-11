//! Discourse adapter for rune-channels.
//!
//! Receives webhook events (post_created) and sends messages via POST /posts.json.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use axum::{
    extract::State,
    http::HeaderMap,
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

const MAX_MSG_LEN: usize = 3000;

/// Discourse webhook adapter.
pub struct DiscourseAdapter {
    base_url: String,
    api_key: Zeroizing<String>,
    api_username: String,
    webhook_port: u16,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

#[derive(Clone)]
struct DiscourseState {
    tx: mpsc::Sender<ChannelMessage>,
}

impl DiscourseAdapter {
    /// Create a new Discourse adapter.
    pub fn new(
        base_url: String,
        api_key: String,
        api_username: String,
        webhook_port: u16,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: Zeroizing::new(api_key),
            api_username,
            webhook_port,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
}

#[async_trait]
impl ChannelAdapter for DiscourseAdapter {
    fn name(&self) -> &str {
        "discourse"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Discourse
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let shutdown_rx = self.shutdown_rx.clone();
        let webhook_port = self.webhook_port;

        let app = Router::new()
            .route("/discourse/webhook", post(discourse_webhook_handler))
            .route("/discourse", post(discourse_webhook_handler))
            .with_state(DiscourseState { tx: tx.clone() });

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
                warn!(error = %e, "Discourse webhook server stopped");
            }
        });

        info!(port = %webhook_port, "Discourse adapter started");
        let stream = ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "Discourse adapter only supports Text content".to_string(),
            )),
        };

        // user.platform_id is topic_id for Discourse; we reply as a new post in that topic
        let topic_id: i64 = user
            .platform_id
            .parse()
            .map_err(|_| ChannelError::Send("invalid topic_id".to_string()))?;

        let url = format!("{}/posts.json", self.base_url);

        let chunks = split_message(&text, MAX_MSG_LEN);
        for chunk in chunks {
            let body = serde_json::json!({
                "raw": chunk,
                "topic_id": topic_id,
                "archetype": "regular"
            });

            let resp = self
                .client
                .post(&url)
                .header("Api-Key", self.api_key.as_str())
                .header("Api-Username", &self.api_username)
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

async fn discourse_webhook_handler(
    State(state): State<DiscourseState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let event = headers
        .get("X-Discourse-Event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if event != "post_created" {
        return Json(serde_json::json!({ "ok": true }));
    }

    let post = match body.get("post").and_then(|p| p.as_object()) {
        Some(p) => p,
        None => return Json(serde_json::json!({ "ok": true })),
    };

    let raw = post
        .get("raw")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if raw.is_empty() {
        return Json(serde_json::json!({ "ok": true }));
    }

    let sender_id = post
        .get("username")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let display_name = sender_id.clone();
    let platform_message_id = post
        .get("id")
        .and_then(|v| v.as_i64())
        .map(|v| v.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let topic_id = post
        .get("topic_id")
        .and_then(|v| v.as_i64())
        .map(|v| v.to_string());

    let mut metadata = HashMap::new();
    if let Some(ref tid) = topic_id {
        metadata.insert("topic_id".to_string(), serde_json::json!(tid));
    }

    let msg = ChannelMessage {
        channel: ChannelType::Discourse,
        platform_message_id,
        sender: ChannelUser {
            platform_id: topic_id.as_ref().map(|s| s.as_str()).unwrap_or(&sender_id).to_string(),
            display_name,
            rune_user: Some(sender_id),
        },
        content: ChannelContent::Text(raw.to_string()),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: true,
        thread_id: None,
        metadata,
    };

    if state.tx.send(msg).await.is_err() {
        warn!("Discourse channel closed, dropping message");
    }

    Json(serde_json::json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discourse_adapter_creation() {
        let adapter = DiscourseAdapter::new(
            "https://forum.example.com".to_string(),
            "api_key".to_string(),
            "system".to_string(),
            19001,
        );
        assert_eq!(adapter.name(), "discourse");
        assert_eq!(adapter.channel_type(), ChannelType::Discourse);
    }
}
