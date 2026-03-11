//! Gitter adapter for rune-channels.
//!
//! Streams messages via GET /v1/rooms/{id}/chatMessages and sends via POST.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::stream::StreamExt;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};
use zeroize::Zeroizing;

const GITTER_API: &str = "https://api.gitter.im/v1";
const GITTER_STREAM: &str = "https://stream.gitter.im/v1";
const MAX_MSG_LEN: usize = 4000;

/// Gitter chat adapter.
pub struct GitterAdapter {
    token: Zeroizing<String>,
    room_id: String,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl GitterAdapter {
    /// Create a new Gitter adapter.
    pub fn new(token: String, room_id: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            token: Zeroizing::new(token),
            room_id,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
}

#[async_trait]
impl ChannelAdapter for GitterAdapter {
    fn name(&self) -> &str {
        "gitter"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Gitter
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let token = self.token.clone();
        let room_id = self.room_id.clone();
        let room_id_log = room_id.clone();
        let client = self.client.clone();
        let shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let stream_url = format!("{}/rooms/{}/chatMessages", GITTER_STREAM, room_id);
            let mut backoff = Duration::from_secs(1);
            const MAX_BACKOFF: Duration = Duration::from_secs(60);

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let req = client
                    .get(&stream_url)
                    .bearer_auth(token.as_str())
                    .build()
                    .map_err(|e| ChannelError::Connection(e.to_string()));

                let req = match req {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(error = %e, "Gitter stream request build failed");
                        tokio::time::sleep(backoff).await;
                        backoff = std::cmp::min(backoff * 2, MAX_BACKOFF);
                        continue;
                    }
                };

                match client.execute(req).await {
                    Ok(resp) => {
                        if !resp.status().is_success() {
                            warn!(status = %resp.status(), "Gitter stream HTTP error");
                            tokio::time::sleep(backoff).await;
                            backoff = std::cmp::min(backoff * 2, MAX_BACKOFF);
                            continue;
                        }

                        backoff = Duration::from_secs(1);
                        let mut stream = resp.bytes_stream();
                        let mut buf = String::new();

                        while let Some(chunk_result) = stream.next().await {
                            if *shutdown_rx.borrow() {
                                break;
                            }

                            match chunk_result {
                                Ok(chunk) => {
                                    buf.push_str(&String::from_utf8_lossy(&chunk));
                                    while let Some(idx) = buf.find('\n') {
                                        let line: String = buf.drain(..=idx).collect();
                                        let line = line.trim();
                                        if !line.is_empty() {
                                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                                                if let Some(msg) = parse_gitter_message(&json, &room_id) {
                                                    if tx.send(msg).await.is_err() {
                                                        return;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!(error = %e, "Gitter stream chunk error");
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Gitter stream connection failed");
                        tokio::time::sleep(backoff).await;
                        backoff = std::cmp::min(backoff * 2, MAX_BACKOFF);
                    }
                }
            }
        });

        info!(room = %room_id_log, "Gitter adapter started");
        let stream = ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(&self, _user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "Gitter adapter only supports Text content".to_string(),
            )),
        };

        let url = format!("{}/rooms/{}/chatMessages", GITTER_API, self.room_id);

        let chunks = split_message(&text, MAX_MSG_LEN);
        for chunk in chunks {
            let body = serde_json::json!({ "text": chunk });

            let resp = self
                .client
                .post(&url)
                .bearer_auth(self.token.as_str())
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

fn parse_gitter_message(json: &serde_json::Value, _room_id: &str) -> Option<ChannelMessage> {
    let id = json.get("id")?.as_str()?.to_string();
    let text = json.get("text").and_then(|v| v.as_str()).unwrap_or("");
    if text.is_empty() {
        return None;
    }

    let user = json.get("fromUser").or_else(|| json.get("user"))?;
    let username = user.get("username").and_then(|v| v.as_str()).unwrap_or("unknown");
    let user_id = user.get("id").and_then(|v| v.as_str()).unwrap_or(username);

    let timestamp = json
        .get("sent")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    Some(ChannelMessage {
        channel: ChannelType::Gitter,
        platform_message_id: id,
        sender: ChannelUser {
            platform_id: user_id.to_string(),
            display_name: username.to_string(),
            rune_user: None,
        },
        content: ChannelContent::Text(text.to_string()),
        target_agent: None,
        timestamp,
        is_group: true,
        thread_id: None,
        metadata: HashMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gitter_adapter_creation() {
        let adapter = GitterAdapter::new("token".to_string(), "room123".to_string());
        assert_eq!(adapter.name(), "gitter");
        assert_eq!(adapter.channel_type(), ChannelType::Gitter);
    }
}
