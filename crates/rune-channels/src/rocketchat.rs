//! Rocket.Chat adapter for rune-channels.
//!
//! Uses Rocket.Chat Realtime API (WebSocket) for receiving messages and REST
//! POST /api/v1/chat.sendMessage for sending.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::{SinkExt, Stream, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use zeroize::Zeroizing;

const ROCKETCHAT_MSG_LIMIT: usize = 4000;

/// Rocket.Chat adapter.
pub struct RocketChatAdapter {
    server_url: String,
    user_id: String,
    auth_token: Zeroizing<String>,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl RocketChatAdapter {
    pub fn new(server_url: String, user_id: String, auth_token: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client");

        Self {
            server_url: server_url.trim_end_matches('/').to_string(),
            user_id,
            auth_token: Zeroizing::new(auth_token),
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

    fn ws_url(&self) -> String {
        let base = self
            .server_url
            .replace("https://", "wss://")
            .replace("http://", "ws://");
        format!("{}/websocket", base)
    }

    fn rest_base(&self) -> String {
        format!("{}/api/v1", self.server_url)
    }
}

#[async_trait]
impl ChannelAdapter for RocketChatAdapter {
    fn name(&self) -> &str {
        "rocketchat"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::RocketChat
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let ws_url = self.ws_url();
        let auth_token = self.auth_token.clone();
        let user_id = self.user_id.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                match connect_async(&ws_url).await {
                    Ok((stream, _)) => {
                        backoff = Duration::from_secs(1);
                        let (mut write, mut read) = stream.split();

                        // Login: send connect + login method
                        let connect_msg = serde_json::json!({
                            "msg": "connect",
                            "version": "1",
                            "support": ["1"]
                        });
                        if let Err(e) = write.send(Message::Text(connect_msg.to_string())).await {
                            warn!(error = %e, "Rocket.Chat connect failed");
                            tokio::time::sleep(backoff).await;
                            backoff = std::cmp::min(backoff * 2, Duration::from_secs(60));
                            continue;
                        }

                        let login_msg = serde_json::json!({
                            "msg": "method",
                            "method": "login",
                            "params": [{ "resume": auth_token.as_str() }],
                            "id": "1"
                        });
                        if let Err(e) = write.send(Message::Text(login_msg.to_string())).await {
                            warn!(error = %e, "Rocket.Chat login failed");
                            tokio::time::sleep(backoff).await;
                            continue;
                        }

                        info!("Rocket.Chat WebSocket connected");

                        loop {
                            tokio::select! {
                                msg = read.next() => {
                                    match msg {
                                        Some(Ok(Message::Text(text))) => {
                                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                                let msg_type = json.get("msg").and_then(|v| v.as_str()).unwrap_or("");
                                                if msg_type == "changed" {
                                                    let collection = json.get("collection").and_then(|v| v.as_str()).unwrap_or("");
                                                    if collection == "stream-room-messages" {
                                                        if let Some(fields) = json.get("fields") {
                                                            if let Some(msg) = parse_rocketchat_message(fields, &user_id) {
                                                                if tx.send(msg).await.is_err() { break; }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        Some(Ok(Message::Close(_))) => { info!("Rocket.Chat WebSocket closed"); break; }
                                        Some(Err(e)) => { warn!(error = %e, "Rocket.Chat WebSocket error"); break; }
                                        None => break,
                                        _ => {}
                                    }
                                }
                                _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Rocket.Chat WebSocket connection failed");
                    }
                }

                if *shutdown_rx.borrow() {
                    break;
                }
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, Duration::from_secs(60));
            }
            drop(tx);
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let room_id = &user.platform_id;
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "Rocket.Chat adapter only supports Text".to_string(),
            )),
        };

        let chunks = split_message(&text, ROCKETCHAT_MSG_LIMIT);
        let url = format!("{}/chat.sendMessage", self.rest_base());

        for chunk in chunks {
            let body = serde_json::json!({
                "message": { "rid": room_id, "msg": chunk }
            });
            let resp = self
                .client
                .post(&url)
                .header("X-Auth-Token", self.auth_token.as_str())
                .header("X-User-Id", &self.user_id)
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let err_text = resp.text().await.unwrap_or_default();
                return Err(ChannelError::Send(err_text));
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

fn parse_rocketchat_message(fields: &serde_json::Value, bot_user_id: &str) -> Option<ChannelMessage> {
    let u = fields.get("u")?;
    let from_id = u.get("_id")?.as_str()?;
    if from_id == bot_user_id {
        return None;
    }
    let display_name = u.get("username").and_then(|v| v.as_str()).unwrap_or(from_id);
    let msg_text = fields.get("msg")?.as_str()?;
    let rid = fields.get("rid").and_then(|v| v.as_str()).unwrap_or("");
    let id = fields.get("_id")?.as_str()?;
    let ts = fields.get("ts").and_then(|v| v.as_str());
    let timestamp = ts
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let mut metadata = HashMap::new();
    metadata.insert("rid".to_string(), serde_json::json!(rid));

    Some(ChannelMessage {
        channel: ChannelType::RocketChat,
        platform_message_id: id.to_string(),
        sender: ChannelUser {
            platform_id: rid.to_string(),
            display_name: display_name.to_string(),
            rune_user: Some(from_id.to_string()),
        },
        content: ChannelContent::Text(msg_text.to_string()),
        target_agent: None,
        timestamp,
        is_group: rid.starts_with('R') || rid.len() > 10,
        thread_id: None,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rocketchat_adapter_creation() {
        let adapter = RocketChatAdapter::new(
            "https://chat.example.com".to_string(),
            "user123".to_string(),
            "token".to_string(),
        );
        assert_eq!(adapter.name(), "rocketchat");
        assert_eq!(adapter.channel_type(), ChannelType::RocketChat);
    }
}
