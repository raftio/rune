//! Feishu/Lark adapter for rune-channels.
//!
//! Spawns HTTP server for event subscription callbacks.
//! send() uses Feishu Bot API.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use axum::{body::Bytes, http::StatusCode, response::IntoResponse, routing::post, Json, Router};
use chrono::{DateTime, Utc};
use futures::Stream;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tracing::info;
use zeroize::Zeroizing;

const FEISHU_API: &str = "https://open.feishu.cn/open-apis";
const MSG_LIMIT: usize = 4000;

/// Feishu/Lark adapter.
pub struct FeishuAdapter {
    app_id: String,
    app_secret: Zeroizing<String>,
    webhook_port: u16,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl FeishuAdapter {
    pub fn new(app_id: String, app_secret: String, webhook_port: u16) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            app_id,
            app_secret: Zeroizing::new(app_secret),
            webhook_port,
            client: reqwest::Client::new(),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

    async fn get_tenant_token(&self) -> Result<String, ChannelError> {
        let url = format!("{}/auth/v3/tenant_access_token/internal", FEISHU_API);
        let body = serde_json::json!({
            "app_id": self.app_id,
            "app_secret": self.app_secret.as_str()
        });
        let resp = self.client.post(&url).json(&body).send().await?;
        let json: serde_json::Value = resp.json().await?;
        let token = json
            .get("tenant_access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::Auth("No tenant_access_token".into()))?;
        Ok(token.to_string())
    }
}

#[async_trait]
impl ChannelAdapter for FeishuAdapter {
    fn name(&self) -> &str {
        "feishu"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Feishu
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let tx_clone = tx.clone();
        let port = self.webhook_port;
        let mut shutdown_rx = self.shutdown_rx.clone();

        let app = Router::new().route(
            "/webhook",
            post(move |body: Bytes| {
                let tx = tx_clone.clone();
                async move {
                    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(body.as_ref()) {
                        if json.get("type").and_then(|v| v.as_str()) == Some("url_verification") {
                            let challenge = json.get("challenge").and_then(|v| v.as_str()).unwrap_or("");
                            return (StatusCode::OK, Json(serde_json::json!({ "challenge": challenge }))).into_response();
                        }
                        if let Some(ev) = json.get("event") {
                            if ev.get("type").and_then(|v| v.as_str()) == Some("im.chat.message.receive_v1") {
                                if let Some(msg) = parse_feishu_event(ev) {
                                    let _ = tx.send(msg).await;
                                }
                            }
                        }
                    }
                    (StatusCode::OK, "OK").into_response()
                }
            }),
        );

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        info!(port = %port, "Feishu webhook server starting");
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let server = axum::serve(listener, app);

        tokio::spawn(async move {
            tokio::select! {
                _ = server => {}
                _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { drop(tx); } }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "Feishu adapter only supports Text".to_string(),
            )),
        };

        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/messages", FEISHU_API);

        for chunk in split_message(&text, MSG_LIMIT) {
            let body = serde_json::json!({
                "receive_id": user.platform_id,
                "msg_type": "text",
                "content": { "text": chunk }
            });
            let resp = self
                .client
                .post(&url)
                .query(&[("receive_id_type", "user_id")])
                .bearer_auth(&token)
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let err = resp.text().await.unwrap_or_default();
                return Err(ChannelError::Send(err));
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

fn parse_feishu_event(ev: &serde_json::Value) -> Option<ChannelMessage> {
    let msg = ev.get("message")?;
    let msg_id = msg.get("message_id")?.as_str()?;
    let sender = ev.get("sender")?.get("sender_id")?.get("user_id")?.as_str()?;
    let content = msg.get("content")?;
    let text = content.get("text")?.as_str()?.to_string();
    let create_time = msg.get("create_time").and_then(|v| v.as_str());
    let timestamp = create_time
        .and_then(|s| s.parse::<i64>().ok())
        .map(|ts| DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now))
        .unwrap_or_else(Utc::now);

    Some(ChannelMessage {
        channel: ChannelType::Feishu,
        platform_message_id: msg_id.to_string(),
        sender: ChannelUser {
            platform_id: sender.to_string(),
            display_name: sender.to_string(),
            rune_user: None,
        },
        content: ChannelContent::Text(text),
        target_agent: None,
        timestamp,
        is_group: ev.get("message")?.get("chat_type")?.as_str() == Some("group"),
        thread_id: ev.get("message")?.get("chat_id")?.as_str().map(String::from),
        metadata: HashMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feishu_adapter_creation() {
        let adapter = FeishuAdapter::new(
            "app_id".to_string(),
            "secret".to_string(),
            8080,
        );
        assert_eq!(adapter.name(), "feishu");
        assert_eq!(adapter.channel_type(), ChannelType::Feishu);
    }
}
