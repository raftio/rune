//! Mattermost WebSocket adapter.
//!
//! Connects to Mattermost via WebSocket at ws(s)://server/api/v4/websocket,
//! authenticates, handles "posted" events, and sends via REST POST /api/v4/posts.

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
use tokio::sync::{mpsc, watch, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use zeroize::Zeroizing;

const MATTERMOST_MSG_LIMIT: usize = 16383;
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Mattermost WebSocket adapter.
pub struct MattermostAdapter {
    server_url: String,
    token: Zeroizing<String>,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Maps user_id -> channel_id for replying.
    reply_context: Arc<RwLock<HashMap<String, String>>>,
    status: Arc<RwLock<ChannelStatus>>,
}

impl MattermostAdapter {
    /// Create a new Mattermost adapter.
    pub fn new(server_url: String, token: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client");

        let ws_url = server_url
            .replace("https://", "wss://")
            .replace("http://", "ws://");
        let _ws_url = ws_url.trim_end_matches('/');

        Self {
            server_url: server_url.trim_end_matches('/').to_string(),
            token: Zeroizing::new(token),
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            reply_context: Arc::new(RwLock::new(HashMap::new())),
            status: Arc::new(RwLock::new(ChannelStatus::default())),
        }
    }

    async fn api_create_post(&self, channel_id: &str, message: &str) -> Result<(), ChannelError> {
        let chunks = split_message(message, MATTERMOST_MSG_LIMIT);
        let url = format!("{}/api/v4/posts", self.server_url);

        for chunk in chunks {
            let body = serde_json::json!({
                "channel_id": channel_id,
                "message": chunk
            });

            let resp = self
                .client
                .post(&url)
                .bearer_auth(self.token.as_str())
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

impl MattermostAdapter {
    fn ws_url_computed(&self) -> String {
        let base = self
            .server_url
            .replace("https://", "wss://")
            .replace("http://", "ws://")
            .trim_end_matches('/')
            .to_string();
        format!("{}/api/v4/websocket", base)
    }
}

/// Parse a Mattermost "posted" event into a ChannelMessage.
fn parse_mattermost_event(
    event: &serde_json::Value,
    bot_user_id: Option<&str>,
) -> Option<ChannelMessage> {
    let event_type = event.get("event")?.as_str()?;
    if event_type != "posted" {
        return None;
    }

    let data = event.get("data")?.as_object()?;
    let post_str = data.get("post")?.as_str()?;
    let post: serde_json::Value = serde_json::from_str(post_str).ok()?;
    let post = post.as_object()?;

    let user_id = post.get("user_id")?.as_str()?;
    if bot_user_id == Some(user_id) {
        return None;
    }

    let message = post.get("message")?.as_str()?.to_string();
    if message.is_empty() {
        return None;
    }

    let channel_id = post.get("channel_id")?.as_str()?.to_string();
    let post_id = post.get("id")?.as_str()?.to_string();
    let create_at = post
        .get("create_at")
        .and_then(|v| v.as_i64())
        .and_then(|ts| DateTime::from_timestamp(ts / 1000, ((ts % 1000) as u32) * 1_000_000))
        .unwrap_or_else(Utc::now);

    let mut metadata = HashMap::new();
    metadata.insert("channel_id".to_string(), serde_json::json!(channel_id.clone()));

    let content = if message.starts_with('/') {
        let parts: Vec<&str> = message.splitn(2, ' ').collect();
        let name = parts[0].strip_prefix('/').unwrap_or("").to_string();
        let args: Vec<String> = if parts.len() > 1 {
            parts[1].split_whitespace().map(String::from).collect()
        } else {
            vec![]
        };
        ChannelContent::Command { name, args }
    } else {
        ChannelContent::Text(message)
    };

    let sender = data.get("sender_name").and_then(|v| v.as_str()).unwrap_or(user_id);

    Some(ChannelMessage {
        channel: ChannelType::Mattermost,
        platform_message_id: post_id,
        sender: ChannelUser {
            platform_id: user_id.to_string(),
            display_name: sender.to_string(),
            rune_user: None,
        },
        content,
        target_agent: None,
        timestamp: create_at,
        is_group: true,
        thread_id: post.get("root_id").and_then(|v| v.as_str()).map(String::from),
        metadata,
    })
}

#[async_trait]
impl ChannelAdapter for MattermostAdapter {
    fn name(&self) -> &str {
        "mattermost"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Mattermost
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let token = self.token.clone();
        let server_url = self.server_url.clone();
        let client = self.client.clone();
        let reply_context = self.reply_context.clone();
        let status = self.status.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let ws_url = self.ws_url_computed();

        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                match run_mattermost_ws(
                    &ws_url,
                    token.as_str(),
                    &client,
                    &server_url,
                    &reply_context,
                    &status,
                    &mut shutdown_rx,
                    &tx,
                )
                .await
                {
                    Ok(()) => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                        backoff = Duration::from_secs(1);
                        warn!("Mattermost WebSocket disconnected, reconnecting...");
                    }
                    Err(e) => {
                        warn!(error = %e, "Mattermost WebSocket error");
                    }
                }

                if *shutdown_rx.borrow() {
                    break;
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
            drop(tx);
        });

        {
            let mut st = self.status.write().await;
            st.connected = true;
            st.started_at = Some(Utc::now());
        }
        info!("Mattermost adapter started");

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let channel_id = {
            let guard = self.reply_context.read().await;
            guard
                .get(&user.platform_id)
                .cloned()
                .ok_or_else(|| ChannelError::Send("No reply context for user".into()))?
        };

        match content {
            ChannelContent::Text(text) => {
                self.api_create_post(&channel_id, &text).await?;
                let mut st = self.status.write().await;
                st.messages_sent += 1;
                Ok(())
            }
            _ => Err(ChannelError::UnsupportedContent(
                "Mattermost adapter only supports Text content".into(),
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

async fn run_mattermost_ws(
    ws_url: &str,
    token: &str,
    _client: &reqwest::Client,
    _server_url: &str,
    reply_context: &Arc<RwLock<HashMap<String, String>>>,
    status: &Arc<RwLock<ChannelStatus>>,
    shutdown_rx: &mut watch::Receiver<bool>,
    tx: &mpsc::Sender<ChannelMessage>,
) -> Result<(), ChannelError> {
    let (stream, _) = connect_async(ws_url)
        .await
        .map_err(|e| ChannelError::Connection(e.to_string()))?;

    let (mut write, mut read) = stream.split();

    // Send auth message: {"seq": 1, "action": "authentication", "data": {"token": "..."}}
    let auth = serde_json::json!({
        "seq": 1,
        "action": "authentication",
        "data": {"token": token}
    });
    write
        .send(Message::Text(auth.to_string()))
        .await
        .map_err(|e| ChannelError::Connection(e.to_string()))?;

    let mut bot_user_id: Option<String> = None;
    let _seq = 2u64;

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let event: serde_json::Value = match serde_json::from_str(&text) {
                            Ok(e) => e,
                            Err(_) => continue,
                        };
                        if event.get("event").and_then(|v| v.as_str()) == Some("hello") {
                            if let Some(broadcast) = event.get("broadcast") {
                                if let Some(id) = broadcast.get("user_id").and_then(|v| v.as_str()) {
                                    bot_user_id = Some(id.to_string());
                                }
                            }
                        }
                        if let Some(msg) = parse_mattermost_event(&event, bot_user_id.as_deref()) {
                            let channel_id = msg.metadata.get("channel_id").and_then(|v| v.as_str()).map(String::from);
                            if let Some(ref cid) = channel_id {
                                reply_context.write().await.insert(msg.sender.platform_id.clone(), cid.clone());
                            }
                            let _ = tx.send(msg).await;
                            status.write().await.messages_received += 1;
                            status.write().await.last_message_at = Some(Utc::now());
                        }
                    }
                    Some(Ok(Message::Close(_))) => return Ok(()),
                    Some(Err(e)) => return Err(ChannelError::Connection(e.to_string())),
                    None => return Ok(()),
                    _ => {}
                }
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mattermost_adapter_creation() {
        let adapter = MattermostAdapter::new(
            "https://mattermost.example.com".to_string(),
            "token".to_string(),
        );
        assert_eq!(adapter.name(), "mattermost");
        assert_eq!(adapter.channel_type(), ChannelType::Mattermost);
    }
}
