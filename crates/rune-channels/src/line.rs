//! LINE Messaging API adapter for rune-channels.
//!
//! Spawns HTTP server for webhook (verifies X-Line-Signature with HMAC-SHA256),
//! send() uses LINE push message API.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use axum::{
    body::Bytes,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Router,
};
use chrono::Utc;
use futures::Stream;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tracing::info;
use zeroize::Zeroizing;

type HmacSha256 = Hmac<Sha256>;
const LINE_API: &str = "https://api.line.me/v2/bot";
const LINE_MSG_LIMIT: usize = 5000;

/// LINE Messaging API adapter.
pub struct LineAdapter {
    channel_secret: Zeroizing<String>,
    channel_access_token: Zeroizing<String>,
    webhook_port: u16,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl LineAdapter {
    pub fn new(
        channel_secret: String,
        channel_access_token: String,
        webhook_port: u16,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let client = reqwest::Client::new();
        Self {
            channel_secret: Zeroizing::new(channel_secret),
            channel_access_token: Zeroizing::new(channel_access_token),
            webhook_port,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

}

fn verify_line_signature(secret: &str, body: &[u8], signature: &str) -> bool {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC key");
    mac.update(body);
    let result = mac.finalize();
    let expected = base64::engine::general_purpose::STANDARD.encode(result.into_bytes());
    if expected.len() != signature.len() {
        return false;
    }
    expected.as_bytes().iter().zip(signature.as_bytes().iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[async_trait]
impl ChannelAdapter for LineAdapter {
    fn name(&self) -> &str {
        "line"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Line
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let tx_clone = tx.clone();
        let secret = self.channel_secret.clone();
        let port = self.webhook_port;
        let mut shutdown_rx = self.shutdown_rx.clone();

        let app = Router::new()
            .route("/webhook", post(move |sig: axum::http::HeaderMap, body: Bytes| {
                let tx = tx_clone.clone();
                let secret = secret.clone();
                async move {
                    let sig_val = sig
                        .get("x-line-signature")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    if sig_val.is_empty() {
                        return (StatusCode::BAD_REQUEST, "Missing signature").into_response();
                    }
                    let body_bytes = body.as_ref();
                    if !verify_line_signature(secret.as_str(), body_bytes, sig_val) {
                        return (StatusCode::UNAUTHORIZED, "Invalid signature").into_response();
                    }
                    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(body_bytes) {
                        if let Some(events) = json.get("events").and_then(|v| v.as_array()) {
                            for ev in events {
                                if ev.get("type").and_then(|v| v.as_str()) == Some("message") {
                                    if let Some(msg) = parse_line_event(ev) {
                                        let _ = tx.send(msg).await;
                                    }
                                }
                            }
                        }
                    }
                    (StatusCode::OK, "OK").into_response()
                }
            }));

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        info!(port = %port, "LINE webhook server starting");
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
        let user_id = &user.platform_id;
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "LINE adapter only supports Text".to_string(),
            )),
        };

        let url = format!("{}/message/push", LINE_API);
        for chunk in split_message(&text, LINE_MSG_LIMIT) {
            let body = serde_json::json!({
                "to": user_id,
                "messages": [{ "type": "text", "text": chunk }]
            });
            let resp = self
                .client
                .post(&url)
                .header(
                    "Authorization",
                    format!("Bearer {}", self.channel_access_token.as_str()),
                )
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

fn parse_line_event(ev: &serde_json::Value) -> Option<ChannelMessage> {
    let user_id = ev.get("source")?.get("userId")?.as_str()?;
    let msg = ev.get("message")?;
    let msg_type = msg.get("type")?.as_str()?;
    let text = if msg_type == "text" {
        msg.get("text")?.as_str()?.to_string()
    } else {
        format!("[{}]", msg_type)
    };
    let id = ev.get("webhookEventId")?.as_str()?;

    Some(ChannelMessage {
        channel: ChannelType::Line,
        platform_message_id: id.to_string(),
        sender: ChannelUser {
            platform_id: user_id.to_string(),
            display_name: user_id.to_string(),
            rune_user: None,
        },
        content: ChannelContent::Text(text),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: ev.get("source")?.get("groupId").is_some(),
        thread_id: ev.get("source")?.get("groupId").and_then(|v| v.as_str()).map(String::from),
        metadata: HashMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_adapter_creation() {
        let adapter = LineAdapter::new(
            "secret".to_string(),
            "token".to_string(),
            8080,
        );
        assert_eq!(adapter.name(), "line");
        assert_eq!(adapter.channel_type(), ChannelType::Line);
    }
}
