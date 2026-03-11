//! Matrix protocol adapter using Client-Server API.
//!
//! Long-polls /sync, parses m.room.message events, and sends via
//! PUT /rooms/{room_id}/send/m.room.message/{txn_id}.

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
use tokio::sync::{mpsc, watch, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};
use zeroize::Zeroizing;

const MAX_MSG_LEN: usize = 65536;
const SYNC_TIMEOUT_MS: u64 = 30000;

/// Matrix Client-Server API adapter.
pub struct MatrixAdapter {
    homeserver_url: String,
    access_token: Zeroizing<String>,
    user_id: String,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    status: Arc<RwLock<ChannelStatus>>,
}

impl MatrixAdapter {
    /// Create a new Matrix adapter.
    pub fn new(
        homeserver_url: String,
        access_token: String,
        user_id: String,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(90))
            .build()
            .expect("reqwest client");

        Self {
            homeserver_url: homeserver_url.trim_end_matches('/').to_string(),
            access_token: Zeroizing::new(access_token),
            user_id,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            status: Arc::new(RwLock::new(ChannelStatus::default())),
        }
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}/_matrix/client/r0{}", self.homeserver_url, path)
    }

    #[allow(dead_code)]
    async fn sync(&self, since: Option<&str>) -> Result<serde_json::Value, ChannelError> {
        let mut url = format!(
            "{}?timeout={}",
            self.api_url("/sync"),
            SYNC_TIMEOUT_MS
        );
        if let Some(s) = since {
            url.push_str(&format!("&since={}", s));
        }

        let resp = self
            .client
            .get(&url)
            .bearer_auth(self.access_token.as_str())
            .send()
            .await
            .map_err(|e| ChannelError::Connection(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ChannelError::Connection(format!("{}: {}", status, text)));
        }

        resp.json()
            .await
            .map_err(|e| ChannelError::InvalidPayload(e.to_string()))
    }

    async fn send_room_message(&self, room_id: &str, text: &str) -> Result<(), ChannelError> {
        let chunks = split_message(text, MAX_MSG_LEN);

        for chunk in chunks {
            let txn_id = uuid::Uuid::new_v4().to_string();
            let path = format!(
                "/rooms/{}/send/m.room.message/{}",
                room_id,
                txn_id
            );
            let url = self.api_url(&path);
            let body = serde_json::json!({
                "msgtype": "m.text",
                "body": chunk
            });

            let resp = self
                .client
                .put(&url)
                .bearer_auth(self.access_token.as_str())
                .json(&body)
                .send()
                .await
                .map_err(|e| ChannelError::Send(e.to_string()))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(ChannelError::Send(format!("{}: {}", status, text)));
            }
        }

        Ok(())
    }
}

/// Parse Matrix sync response into ChannelMessage(s).
fn parse_matrix_sync(
    sync: &serde_json::Value,
    bot_user_id: &str,
) -> (Vec<ChannelMessage>, Option<String>) {
    let mut messages = Vec::new();
    let next_batch = sync.get("next_batch").and_then(|v| v.as_str()).map(String::from);

    let rooms = sync.get("rooms").and_then(|r| r.get("join")).and_then(|j| j.as_object());
    let rooms = match rooms {
        Some(r) => r,
        None => return (messages, next_batch),
    };

    for (room_id, room) in rooms {
        let Some(timeline) = room.get("timeline").and_then(|t| t.get("events")).and_then(|e| e.as_array()) else {
            continue;
        };
        for ev in timeline {
            if ev.get("type").and_then(|t| t.as_str()) != Some("m.room.message") {
                continue;
            }
            let Some(sender) = ev.get("sender").and_then(|s| s.as_str()) else {
                continue;
            };
            if sender == bot_user_id {
                continue;
            }
            let Some(content) = ev.get("content").and_then(|c| c.as_object()) else {
                continue;
            };
            let Some(body) = content.get("body").and_then(|b| b.as_str()).map(String::from) else {
                continue;
            };
            let Some(event_id) = ev.get("event_id").and_then(|e| e.as_str()).map(String::from) else {
                continue;
            };

            let origin_server_ts = ev
                .get("origin_server_ts")
                .and_then(|v| v.as_i64())
                .and_then(|ts| DateTime::from_timestamp(ts / 1000, ((ts % 1000) as u32) * 1_000_000))
                .unwrap_or_else(Utc::now);

            let display_name = content
                .get("displayname")
                .and_then(|v| v.as_str())
                .unwrap_or(sender)
                .to_string();

            let content_enum = if body.starts_with("!") || body.starts_with("/") {
                let parts: Vec<&str> = body.splitn(2, ' ').collect();
                let cmd = parts[0].trim_start_matches('!').trim_start_matches('/');
                let name = cmd.to_string();
                let args: Vec<String> = if parts.len() > 1 {
                    parts[1].split_whitespace().map(String::from).collect()
                } else {
                    vec![]
                };
                ChannelContent::Command { name, args }
            } else {
                ChannelContent::Text(body)
            };

            let mut metadata = HashMap::new();
            metadata.insert("room_id".to_string(), serde_json::json!(room_id));
            metadata.insert("event_id".to_string(), serde_json::json!(event_id));

            messages.push(ChannelMessage {
                channel: ChannelType::Matrix,
                platform_message_id: event_id,
                sender: ChannelUser {
                    platform_id: format!("{}|{}", room_id, sender),
                    display_name,
                    rune_user: None,
                },
                content: content_enum,
                target_agent: None,
                timestamp: origin_server_ts,
                is_group: true,
                thread_id: content
                    .get("m.thread")
                    .and_then(|t| t.get("event_id"))
                    .and_then(|e| e.as_str())
                    .map(String::from)
                    .or_else(|| Some(room_id.to_string())),
                metadata,
            });
        }
    }

    (messages, next_batch)
}

#[async_trait]
impl ChannelAdapter for MatrixAdapter {
    fn name(&self) -> &str {
        "matrix"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Matrix
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let client = self.client.clone();
        let homeserver_url = self.homeserver_url.clone();
        let access_token = self.access_token.clone();
        let user_id = self.user_id.clone();
        let status = self.status.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let mut since: Option<String> = None;
            let mut backoff = Duration::from_secs(1);
            const MAX_BACKOFF: Duration = Duration::from_secs(60);

            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() { break; }
                    }
                    _ = tokio::time::sleep(Duration::ZERO) => {}
                }
                if *shutdown_rx.borrow() {
                    break;
                }

                let url = format!("{}/_matrix/client/r0/sync?timeout={}{}",
                    homeserver_url, SYNC_TIMEOUT_MS,
                    since.as_ref().map(|s| format!("&since={}", s)).unwrap_or_default());
                let resp = match client
                    .get(&url)
                    .bearer_auth(access_token.as_str())
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(error = %e, "Matrix sync failed");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue;
                    }
                };

                let sync: serde_json::Value = match resp.json().await {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(error = %e, "Matrix sync parse failed");
                        continue;
                    }
                };

                backoff = Duration::from_secs(1);
                since = sync.get("next_batch").and_then(|v| v.as_str()).map(String::from);

                let (msgs, _) = parse_matrix_sync(&sync, &user_id);
                for msg in msgs {
                    let _ = tx.send(msg).await;
                    let mut st = status.write().await;
                    st.messages_received += 1;
                    st.last_message_at = Some(Utc::now());
                }
            }

            let mut st = status.write().await;
            st.connected = false;
        });

        {
            let mut st = self.status.write().await;
            st.connected = true;
            st.started_at = Some(Utc::now());
        }
        info!("Matrix adapter started");

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let parts: Vec<&str> = user.platform_id.splitn(2, '|').collect();
        let room_id = parts.first().copied().unwrap_or(&user.platform_id);

        match content {
            ChannelContent::Text(text) => {
                self.send_room_message(room_id, &text).await?;
                let mut st = self.status.write().await;
                st.messages_sent += 1;
                Ok(())
            }
            _ => Err(ChannelError::UnsupportedContent(
                "Matrix adapter only supports Text content".into(),
            )),
        }
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        self.status.blocking_read().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matrix_adapter_creation() {
        let adapter = MatrixAdapter::new(
            "https://matrix.example.com".to_string(),
            "syt_xxx".to_string(),
            "@bot:example.com".to_string(),
        );
        assert_eq!(adapter.name(), "matrix");
        assert_eq!(adapter.channel_type(), ChannelType::Matrix);
    }
}
