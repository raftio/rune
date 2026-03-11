//! Guilded adapter for rune-channels.
//!
//! Connects to Guilded WebSocket for ChatMessageCreated events.
//! Sends via REST POST /channels/{id}/messages.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::{Stream, SinkExt, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tracing::{info, warn};
use zeroize::Zeroizing;

const GUILDED_WS: &str = "wss://api.guilded.gg/socket.io/?jwt=";
const GUILDED_API_DEFAULT: &str = "https://api.guilded.gg/v1";
const MAX_MSG_LEN: usize = 4000;

/// Guilded adapter.
pub struct GuildedAdapter {
    token: Zeroizing<String>,
    api_url: String,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    status: Arc<RwLock<ChannelStatus>>,
    /// Maps author platform_id -> channel_id for send().
    user_to_channel: Arc<RwLock<HashMap<String, String>>>,
}

impl GuildedAdapter {
    /// Create a new Guilded adapter.
    pub fn new(token: String, api_url: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            token: Zeroizing::new(token),
            api_url: api_url.unwrap_or_else(|| GUILDED_API_DEFAULT.to_string()),
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            status: Arc::new(RwLock::new(ChannelStatus::default())),
            user_to_channel: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token.as_str())
    }
}

#[async_trait]
impl ChannelAdapter for GuildedAdapter {
    fn name(&self) -> &str {
        "guilded"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Guilded
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let token = self.token.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let status = self.status.clone();
        let user_to_channel = self.user_to_channel.clone();

        tokio::spawn(async move {
            let ws_url = format!("{}{}", GUILDED_WS, token.as_str());
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                match connect_async(&ws_url).await {
                    Ok((ws_stream, _)) => {
                        status.write().await.connected = true;
                        status.write().await.started_at = Some(Utc::now());
                        info!("Guilded WebSocket connected");

                        let (mut write, mut read) = ws_stream.split();
                        let tx_clone = tx.clone();

                        loop {
                            tokio::select! {
                                Some(msg) = read.next() => {
                                    match msg {
                                        Ok(WsMessage::Text(text)) => {
                                            if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&text) {
                                                if let Some((msg, channel_id)) = parse_guilded_message(&payload) {
                                                    user_to_channel.write().await.insert(msg.sender.platform_id.clone(), channel_id.clone());
                                                    if tx_clone.send(msg).await.is_err() {
                                                        break;
                                                    }
                                                    status.write().await.messages_received += 1;
                                                    status.write().await.last_message_at = Some(Utc::now());
                                                }
                                            }
                                        }
                                        Ok(WsMessage::Close(_)) => break,
                                        Err(e) => {
                                            warn!(error = %e, "Guilded WebSocket error");
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                                _ = shutdown_rx.changed() => {
                                    if *shutdown_rx.borrow() {
                                        let _ = write.close().await;
                                        break;
                                    }
                                }
                            }
                        }

                        status.write().await.connected = false;
                    }
                    Err(e) => {
                        warn!(error = %e, "Guilded WebSocket connection failed");
                        status.write().await.last_error = Some(e.to_string());
                        status.write().await.connected = false;
                    }
                }

                if *shutdown_rx.borrow() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let channel_id = {
            let map = self.user_to_channel.read().await;
            map.get(&user.platform_id).cloned()
        };

        let channel_id = channel_id
            .ok_or_else(|| ChannelError::Send("no channel mapping for user".to_string()))?;
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            ChannelContent::Image { url, caption } => {
                format!("{} {}", url, caption.as_deref().unwrap_or("")).trim().to_string()
            }
            _ => {
                return Err(ChannelError::UnsupportedContent(
                    "Guilded adapter only supports Text and Image".to_string(),
                ))
            }
        };

        let url = format!("{}/channels/{}/messages", self.api_url, &channel_id);
        let chunks = split_message(&text, MAX_MSG_LEN);

        for chunk in chunks {
            let body = serde_json::json!({
                "content": chunk,
                "isPrivate": false,
            });

            let resp = self
                .client
                .post(&url)
                .header("Authorization", self.auth_header())
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

        self.status.write().await.messages_sent += 1;
        Ok(())
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        self.status.blocking_read().clone()
    }
}

fn parse_guilded_message(payload: &serde_json::Value) -> Option<(ChannelMessage, String)> {
    let t = payload.get("t").and_then(|v| v.as_str())?;
    if t != "ChatMessageCreated" {
        return None;
    }
    let d = payload.get("d")?.as_object()?;
    let message = d.get("message")?.as_object()?;
    let id = message.get("id")?.as_str()?;
    let content = message.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let channel_id = message.get("channelId")?.as_str()?;
    let created_at = message
        .get("createdAt")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let created_by = message.get("createdBy").and_then(|v| v.as_str()).unwrap_or("");
    let mut metadata = HashMap::new();
    metadata.insert(
        "channel_id".to_string(),
        serde_json::json!(channel_id),
    );

    Some((
        ChannelMessage {
            channel: ChannelType::Guilded,
            platform_message_id: id.to_string(),
            sender: ChannelUser {
                platform_id: created_by.to_string(),
                display_name: created_by.to_string(),
                rune_user: Some(created_by.to_string()),
            },
            content: ChannelContent::Text(content.to_string()),
            target_agent: None,
            timestamp: created_at,
            is_group: true,
            thread_id: Some(channel_id.to_string()),
            metadata,
        },
        channel_id.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guilded_adapter_creation() {
        let adapter = GuildedAdapter::new(
            "test-token".to_string(),
            Some("https://api.guilded.gg/v1".to_string()),
        );
        assert_eq!(adapter.name(), "guilded");
        assert_eq!(adapter.channel_type(), ChannelType::Guilded);
    }
}
