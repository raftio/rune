//! Mastodon adapter for rune-channels.
//!
//! Uses streaming API GET /api/v1/streaming/user for receiving.
//! send() uses POST /api/v1/statuses.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::{Stream, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use zeroize::Zeroizing;

const MSG_LIMIT: usize = 500;

/// Mastodon adapter.
pub struct MastodonAdapter {
    instance_url: String,
    access_token: Zeroizing<String>,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl MastodonAdapter {
    pub fn new(instance_url: String, access_token: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let instance = instance_url.trim_end_matches('/').to_string();
        Self {
            instance_url: instance,
            access_token: Zeroizing::new(access_token),
            client: reqwest::Client::new(),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

    fn ws_url(&self) -> String {
        let base = self
            .instance_url
            .replace("https://", "wss://")
            .replace("http://", "ws://");
        format!("{}/api/v1/streaming?stream=user&access_token={}", base, self.access_token.as_str())
    }

    fn api_base(&self) -> String {
        format!("{}/api/v1", self.instance_url)
    }
}

#[async_trait]
impl ChannelAdapter for MastodonAdapter {
    fn name(&self) -> &str {
        "mastodon"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Mastodon
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let ws_url = self.ws_url();
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
                        let (mut _write, mut read) = stream.split();
                        info!("Mastodon streaming connected");

                        loop {
                            tokio::select! {
                                msg = read.next() => {
                                    match msg {
                                        Some(Ok(Message::Text(line))) => {
                                            let data_line = line.lines().find(|l| l.starts_with("data: "));
                                            if let Some(d) = data_line {
                                                let json_str = d.strip_prefix("data: ").unwrap_or(d);
                                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
                                                    let msg = if line.contains("event: notification") {
                                                        parse_mastodon_notification(&json)
                                                    } else if line.contains("event: update") {
                                                        parse_mastodon_status(&json)
                                                    } else {
                                                        None
                                                    };
                                                    if let Some(m) = msg {
                                                        if tx.send(m).await.is_err() { break; }
                                                    }
                                                }
                                            }
                                        }
                                        Some(Ok(Message::Close(_))) => break,
                                        Some(Err(e)) => { warn!(error = %e, "Mastodon stream error"); break; }
                                        None => break,
                                        _ => {}
                                    }
                                }
                                _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Mastodon streaming connection failed");
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

    async fn send(&self, _user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "Mastodon adapter only supports Text".to_string(),
            )),
        };

        let url = format!("{}/statuses", self.api_base());
        for chunk in split_message(&text, MSG_LIMIT) {
            let body = serde_json::json!({
                "status": chunk,
                "visibility": "direct"
            });
            let resp = self
                .client
                .post(&url)
                .bearer_auth(self.access_token.as_str())
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

fn parse_mastodon_notification(json: &serde_json::Value) -> Option<ChannelMessage> {
    let id = json
        .get("id")
        .and_then(|v| v.as_str().map(String::from).or_else(|| v.as_i64().map(|i| i.to_string())))
        .unwrap_or_else(|| "0".to_string());
    let account = json.get("account")?;
    let acct = account.get("acct")?.as_str()?;
    let display_name = account.get("display_name").and_then(|v| v.as_str()).unwrap_or(acct);
    let content = json.get("status").and_then(|s| s.get("content")).and_then(|c| c.as_str()).unwrap_or("").to_string();
    let created = json.get("created_at").and_then(|v| v.as_str());
    let timestamp = created
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    Some(ChannelMessage {
        channel: ChannelType::Mastodon,
        platform_message_id: id,
        sender: ChannelUser {
            platform_id: acct.to_string(),
            display_name: display_name.to_string(),
            rune_user: None,
        },
        content: ChannelContent::Text(content),
        target_agent: None,
        timestamp,
        is_group: false,
        thread_id: None,
        metadata: HashMap::new(),
    })
}

fn parse_mastodon_status(json: &serde_json::Value) -> Option<ChannelMessage> {
    let id = json
        .get("id")
        .and_then(|v| v.as_str().map(String::from).or_else(|| v.as_i64().map(|i| i.to_string())))
        .unwrap_or_else(|| "0".to_string());
    let account = json.get("account")?;
    let acct = account.get("acct")?.as_str()?;
    let display_name = account.get("display_name").and_then(|v| v.as_str()).unwrap_or(acct);
    let content = json.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let created = json.get("created_at").and_then(|v| v.as_str());
    let timestamp = created
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    Some(ChannelMessage {
        channel: ChannelType::Mastodon,
        platform_message_id: id,
        sender: ChannelUser {
            platform_id: acct.to_string(),
            display_name: display_name.to_string(),
            rune_user: None,
        },
        content: ChannelContent::Text(content),
        target_agent: None,
        timestamp,
        is_group: false,
        thread_id: None,
        metadata: HashMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mastodon_adapter_creation() {
        let adapter = MastodonAdapter::new(
            "https://mastodon.social".to_string(),
            "token".to_string(),
        );
        assert_eq!(adapter.name(), "mastodon");
        assert_eq!(adapter.channel_type(), ChannelType::Mastodon);
    }
}
