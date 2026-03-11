//! Revolt adapter for rune-channels.
//!
//! Connects to WebSocket, authenticates, handles Message events.
//! send() uses REST POST /channels/{id}/messages.

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
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use zeroize::Zeroizing;

const MSG_LIMIT: usize = 2000;

/// Revolt adapter.
pub struct RevoltAdapter {
    token: Zeroizing<String>,
    api_url: String,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl RevoltAdapter {
    pub fn new(token: String, api_url: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let api = api_url.trim_end_matches('/').to_string();
        Self {
            token: Zeroizing::new(token),
            api_url: api,
            client: reqwest::Client::new(),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

    fn ws_url(&self) -> String {
        let base = self
            .api_url
            .replace("https://", "wss://")
            .replace("http://", "ws://");
        format!("{}/", base)
    }

    fn api_base(&self) -> String {
        self.api_url.clone()
    }
}

#[async_trait]
impl ChannelAdapter for RevoltAdapter {
    fn name(&self) -> &str {
        "revolt"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Revolt
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let ws_url = self.ws_url();
        let token = self.token.clone();
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

                        let auth = serde_json::json!({
                            "type": "Authenticate",
                            "token": token.as_str()
                        });
                        if write.send(Message::Text(auth.to_string())).await.is_err() {
                            warn!("Revolt auth send failed");
                            tokio::time::sleep(backoff).await;
                            continue;
                        }

                        info!("Revolt WebSocket connected");

                        loop {
                            tokio::select! {
                                msg = read.next() => {
                                    match msg {
                                        Some(Ok(Message::Text(line))) => {
                                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                                                let msg_type = json.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                                if msg_type == "Message" {
                                                    if let Some(data) = json.get("data") {
                                                        if let Some(m) = parse_revolt_message(data) {
                                                            if tx.send(m).await.is_err() { break; }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        Some(Ok(Message::Close(_))) => break,
                                        Some(Err(e)) => { warn!(error = %e, "Revolt WebSocket error"); break; }
                                        None => break,
                                        _ => {}
                                    }
                                }
                                _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Revolt WebSocket connection failed");
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
        let channel_id = &user.platform_id;
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "Revolt adapter only supports Text".to_string(),
            )),
        };

        let url = format!("{}/channels/{}/messages", self.api_base(), channel_id);
        for chunk in split_message(&text, MSG_LIMIT) {
            let body = serde_json::json!({ "content": chunk });
            let resp = self
                .client
                .post(&url)
                .header("x-bot-token", self.token.as_str())
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

fn parse_revolt_message(data: &serde_json::Value) -> Option<ChannelMessage> {
    let id = data.get("_id")?.as_str()?;
    let channel = data.get("channel")?.as_str()?;
    let author = data.get("author")?.as_str()?;
    let content = data.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let created = data.get("created_at").and_then(|v| v.as_str());
    let timestamp = created
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let mut metadata = HashMap::new();
    metadata.insert("channel_id".to_string(), serde_json::json!(channel));

    Some(ChannelMessage {
        channel: ChannelType::Revolt,
        platform_message_id: id.to_string(),
        sender: ChannelUser {
            platform_id: channel.to_string(),
            display_name: author.to_string(),
            rune_user: Some(author.to_string()),
        },
        content: ChannelContent::Text(content),
        target_agent: None,
        timestamp,
        is_group: true,
        thread_id: Some(channel.to_string()),
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_revolt_adapter_creation() {
        let adapter = RevoltAdapter::new(
            "token".to_string(),
            "https://api.revolt.chat".to_string(),
        );
        assert_eq!(adapter.name(), "revolt");
        assert_eq!(adapter.channel_type(), ChannelType::Revolt);
    }
}
