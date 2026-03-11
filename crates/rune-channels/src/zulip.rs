//! Zulip adapter for rune-channels.
//!
//! Uses GET /api/v1/events (long-poll) with queue_id from register for receiving,
//! and POST /api/v1/messages for sending.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};
use zeroize::Zeroizing;

const ZULIP_MSG_LIMIT: usize = 10000;

/// Zulip adapter.
pub struct ZulipAdapter {
    server_url: String,
    email: String,
    api_key: Zeroizing<String>,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl ZulipAdapter {
    pub fn new(server_url: String, email: String, api_key: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(90))
            .build()
            .expect("reqwest client");

        Self {
            server_url: server_url.trim_end_matches('/').to_string(),
            email,
            api_key: Zeroizing::new(api_key),
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

    fn api_base(&self) -> String {
        format!("{}/api/v1", self.server_url)
    }
}

#[async_trait]
impl ChannelAdapter for ZulipAdapter {
    fn name(&self) -> &str {
        "zulip"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Zulip
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let url = format!("{}/register", self.api_base());
        let resp = self
            .client
            .post(&url)
            .basic_auth(&self.email, Some(self.api_key.as_str()))
            .json(&serde_json::json!({ "event_types": ["message"] }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(ChannelError::Auth(err));
        }

        let reg: serde_json::Value = resp.json().await?;
        let queue_id = reg
            .get("queue_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::InvalidPayload("register missing queue_id".into()))?
            .to_string();

        info!(queue_id = %queue_id, "Zulip adapter registered");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let client = self.client.clone();
        let email = self.email.clone();
        let api_key = self.api_key.clone();
        let api_base = self.api_base();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let events_url = format!("{}/events", api_base);
                let params = [
                    ("queue_id", queue_id.as_str()),
                    ("last_event_id", "-1"),
                ];

                match client
                    .get(&events_url)
                    .basic_auth(&email, Some(api_key.as_str()))
                    .query(&params)
                    .send()
                    .await
                {
                    Ok(resp) => {
                        let json: serde_json::Value = match resp.json().await {
                            Ok(j) => j,
                            Err(e) => {
                                warn!(error = %e, "Zulip events parse failed");
                                tokio::time::sleep(Duration::from_secs(1)).await;
                                continue;
                            }
                        };

                        if let Some(events) = json.get("events").and_then(|v| v.as_array()) {
                            for ev in events {
                                if ev.get("type").and_then(|v| v.as_str()) != Some("message") {
                                    continue;
                                }
                                if let Some(msg) = ev.get("message").and_then(|m| {
                                    parse_zulip_message(m)
                                }) {
                                    if tx.send(msg).await.is_err() {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Zulip events request failed");
                    }
                }

                tokio::select! {
                    _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => { }
                }
            }
            drop(tx);
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "Zulip adapter only supports Text".to_string(),
            )),
        };

        // user.platform_id encodes type:id (e.g. stream:123 or email:user@host)
        let parts: Vec<&str> = user.platform_id.splitn(2, ':').collect();
        let (to_type, to_id) = if parts.len() >= 2 {
            (parts[0].to_string(), parts[1].to_string())
        } else {
            ("stream".to_string(), user.platform_id.clone())
        };

        let url = format!("{}/messages", self.api_base());
        let chunks = split_message(&text, ZULIP_MSG_LIMIT);

        for chunk in chunks {
            let body = if to_type == "stream" {
            serde_json::json!({
                "type": "stream",
                "to": [to_id],
                "topic": "general",
                "content": chunk
            })
        } else {
            serde_json::json!({
                "type": "private",
                "to": [to_id],
                "content": chunk
            })
        };

            let resp = self
                .client
                .post(&url)
                .basic_auth(&self.email, Some(self.api_key.as_str()))
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

fn parse_zulip_message(msg: &serde_json::Value) -> Option<ChannelMessage> {
    let sender_id = msg.get("sender_id")?.as_i64()?;
    let sender_email = msg.get("sender_email")?.as_str()?;
    let content = msg.get("content")?.as_str()?.to_string();
    let id = msg.get("id")?.as_i64()?.to_string();
    let timestamp = msg
        .get("timestamp")
        .and_then(|v| v.as_i64())
        .map(|ts| DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now))
        .unwrap_or_else(Utc::now);

    let (platform_id, is_group) = if let Some(recipient) = msg.get("display_recipient") {
        if recipient.is_array() {
            let stream_id = msg.get("stream_id").and_then(|v| v.as_i64()).unwrap_or(0);
            (format!("stream:{}", stream_id), true)
        } else {
            (format!("private:{}", sender_email), false)
        }
    } else {
        (format!("private:{}", sender_email), false)
    };

    let mut metadata = HashMap::new();
    if let Some(topic) = msg.get("subject").and_then(|v| v.as_str()) {
        metadata.insert("topic".to_string(), serde_json::json!(topic));
    }

    Some(ChannelMessage {
        channel: ChannelType::Zulip,
        platform_message_id: id,
        sender: ChannelUser {
            platform_id,
            display_name: msg
                .get("sender_full_name")
                .and_then(|v| v.as_str())
                .unwrap_or(sender_email)
                .to_string(),
            rune_user: Some(sender_id.to_string()),
        },
        content: ChannelContent::Text(content),
        target_agent: None,
        timestamp,
        is_group,
        thread_id: msg.get("subject").and_then(|v| v.as_str()).map(String::from),
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zulip_adapter_creation() {
        let adapter = ZulipAdapter::new(
            "https://chat.zulip.com".to_string(),
            "bot@example.com".to_string(),
            "api_key".to_string(),
        );
        assert_eq!(adapter.name(), "zulip");
        assert_eq!(adapter.channel_type(), ChannelType::Zulip);
    }
}
