//! Gotify adapter for rune-channels.
//!
//! Connects to WebSocket /stream for incoming messages and sends via POST /message.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::{Stream, stream::StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use zeroize::Zeroizing;

const MAX_MSG_LEN: usize = 4000;

/// Gotify push notification adapter.
pub struct GotifyAdapter {
    server_url: String,
    app_token: Zeroizing<String>,
    client_token: Zeroizing<String>,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl GotifyAdapter {
    /// Create a new Gotify adapter.
    pub fn new(server_url: String, app_token: String, client_token: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            server_url: server_url.trim_end_matches('/').to_string(),
            app_token: Zeroizing::new(app_token),
            client_token: Zeroizing::new(client_token),
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
}

#[async_trait]
impl ChannelAdapter for GotifyAdapter {
    fn name(&self) -> &str {
        "gotify"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Gotify
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let client_token = self.client_token.clone();
        let server_url = self.server_url.clone();
        let shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let ws_url = format!(
                "{}/stream?token={}",
                server_url.replace("https://", "wss://").replace("http://", "ws://"),
                client_token.as_str()
            );
            let mut backoff = Duration::from_secs(1);
            const MAX_BACKOFF: Duration = Duration::from_secs(60);

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                match connect_async(&ws_url).await {
                    Ok((stream, _)) => {
                        backoff = Duration::from_secs(1);
                        let (mut _write, mut read) = stream.split();

                        while let Some(msg_result) = read.next().await {
                            if *shutdown_rx.borrow() {
                                break;
                            }

                            match msg_result {
                                Ok(Message::Text(text)) => {
                                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                        if let Some(msg) = parse_gotify_message(&json) {
                                            if tx.send(msg).await.is_err() {
                                                break;
                                            }
                                        }
                                    }
                                }
                                Ok(Message::Close(_)) => break,
                                Ok(Message::Ping(_) | Message::Pong(_)) => {}
                                Err(e) => {
                                    warn!(error = %e, "Gotify WebSocket error");
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Gotify WebSocket connection failed");
                    }
                }

                if *shutdown_rx.borrow() {
                    break;
                }
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, MAX_BACKOFF);
            }
        });

        info!(server = %self.server_url, "Gotify adapter started");
        let stream = ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "Gotify adapter only supports Text content".to_string(),
            )),
        };

        let url = format!("{}/message", self.server_url);

        let chunks = split_message(&text, MAX_MSG_LEN);
        for chunk in chunks {
            let body = serde_json::json!({
                "message": chunk,
                "title": user.display_name,
                "priority": 5
            });

            let resp = self
                .client
                .post(&url)
                .query(&[("token", self.app_token.as_str())])
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

fn parse_gotify_message(json: &serde_json::Value) -> Option<ChannelMessage> {
    let id = json
        .get("id")
        .and_then(|v| v.as_i64())
        .map(|v| v.to_string())
        .or_else(|| json.get("id").and_then(|v| v.as_str()).map(String::from))
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let message = json.get("message").and_then(|v| v.as_str()).unwrap_or("");
    if message.is_empty() {
        return None;
    }

    let title = json.get("title").and_then(|v| v.as_str()).unwrap_or("Gotify");
    let app_id = json
        .get("appid")
        .and_then(|v| v.as_i64())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "0".to_string());

    Some(ChannelMessage {
        channel: ChannelType::Gotify,
        platform_message_id: id,
        sender: ChannelUser {
            platform_id: app_id,
            display_name: title.to_string(),
            rune_user: None,
        },
        content: ChannelContent::Text(message.to_string()),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: false,
        thread_id: None,
        metadata: HashMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gotify_adapter_creation() {
        let adapter = GotifyAdapter::new(
            "https://gotify.example.com".to_string(),
            "app_token".to_string(),
            "client_token".to_string(),
        );
        assert_eq!(adapter.name(), "gotify");
        assert_eq!(adapter.channel_type(), ChannelType::Gotify);
    }
}
