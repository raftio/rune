//! ntfy.sh adapter for rune-channels.
//!
//! Subscribes to a topic via SSE stream (GET /topic/sse) and publishes via POST /topic.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
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

const DEFAULT_SERVER: &str = "https://ntfy.sh";
const MAX_MSG_LEN: usize = 4000;

/// ntfy.sh pub-sub adapter.
pub struct NtfyAdapter {
    server_url: String,
    topic: String,
    auth_token: Option<Zeroizing<String>>,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl NtfyAdapter {
    /// Create a new ntfy adapter.
    pub fn new(
        topic: String,
        server_url: Option<String>,
        auth_token: Option<String>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(86400))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            server_url: server_url
                .unwrap_or_else(|| DEFAULT_SERVER.to_string())
                .trim_end_matches('/')
                .to_string(),
            topic,
            auth_token: auth_token.map(Zeroizing::new),
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
}

#[async_trait]
impl ChannelAdapter for NtfyAdapter {
    fn name(&self) -> &str {
        "ntfy"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Ntfy
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let topic = self.topic.clone();
        let server_url = self.server_url.clone();
        let auth_token = self.auth_token.clone();
        let client = self.client.clone();
        let shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let sse_url = format!("{}/{}/sse", server_url, topic);
            let mut backoff = Duration::from_secs(1);
            const MAX_BACKOFF: Duration = Duration::from_secs(60);

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let mut req = client.get(&sse_url);
                if let Some(ref token) = auth_token {
                    req = req.header("Authorization", format!("Bearer {}", token.as_str()));
                }

                match req.send().await {
                    Ok(resp) => {
                        if !resp.status().is_success() {
                            warn!(status = %resp.status(), "ntfy SSE subscribe failed");
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
                                        if line.starts_with("data: ") {
                                            let json_str = line.trim_start_matches("data: ");
                                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
                                                if let Some(msg) = parse_ntfy_message(&json, &topic) {
                                                    if tx.send(msg).await.is_err() {
                                                        return;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!(error = %e, "ntfy SSE stream error");
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "ntfy SSE connection failed");
                    }
                }

                if *shutdown_rx.borrow() {
                    break;
                }
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, MAX_BACKOFF);
            }
        });

        info!(topic = %self.topic, server = %self.server_url, "ntfy adapter started");
        let stream = ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(&self, _user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "ntfy adapter only supports Text content".to_string(),
            )),
        };

        let url = format!("{}/{}", self.server_url, self.topic);

        let chunks = split_message(&text, MAX_MSG_LEN);
        for chunk in chunks {
            let mut req = self
                .client
                .post(&url)
                .header("Content-Type", "text/plain")
                .body(chunk.to_string());

            if let Some(ref token) = self.auth_token {
                req = req.header("Authorization", format!("Bearer {}", token.as_str()));
            }

            let resp = req.send().await?;

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

fn parse_ntfy_message(json: &serde_json::Value, topic: &str) -> Option<ChannelMessage> {
    let event = json.get("event").and_then(|v| v.as_str()).unwrap_or("");
    if event != "message" {
        return None;
    }

    let id = json.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let message = json.get("message").and_then(|v| v.as_str()).unwrap_or("");
    if message.is_empty() {
        return None;
    }

    let time = json
        .get("time")
        .and_then(|v| v.as_i64())
        .and_then(|t| DateTime::from_timestamp(t, 0))
        .unwrap_or_else(Utc::now);
    let title = json.get("title").and_then(|v| v.as_str()).unwrap_or("ntfy");

    Some(ChannelMessage {
        channel: ChannelType::Ntfy,
        platform_message_id: id,
        sender: ChannelUser {
            platform_id: topic.to_string(),
            display_name: title.to_string(),
            rune_user: None,
        },
        content: ChannelContent::Text(message.to_string()),
        target_agent: None,
        timestamp: time,
        is_group: true,
        thread_id: None,
        metadata: HashMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ntfy_adapter_creation() {
        let adapter = NtfyAdapter::new("mytopic".to_string(), None, None);
        assert_eq!(adapter.name(), "ntfy");
        assert_eq!(adapter.channel_type(), ChannelType::Ntfy);
    }

    #[test]
    fn test_ntfy_adapter_with_auth() {
        let adapter = NtfyAdapter::new(
            "privatetopic".to_string(),
            Some("https://ntfy.example.com".to_string()),
            Some("token".to_string()),
        );
        assert_eq!(adapter.name(), "ntfy");
    }
}
