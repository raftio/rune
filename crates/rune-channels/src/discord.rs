//! Discord WebSocket gateway adapter.
//!
//! Connects to the Discord Gateway, receives MESSAGE_CREATE events,
//! and sends replies via the REST API.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::{SinkExt, Stream, StreamExt};
use std::collections::HashMap;
use tokio_stream::wrappers::ReceiverStream;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tracing::{debug, error, info, warn};
use zeroize::Zeroizing;

const DISCORD_API: &str = "https://discord.com/api/v10";
const DISCORD_GATEWAY: &str = "wss://gateway.discord.gg/?v=10&encoding=json";
const DISCORD_MSG_LIMIT: usize = 2000;

/// Discord Gateway intents: GUILD_MESSAGES | DIRECT_MESSAGES | MESSAGE_CONTENT
const DISCORD_INTENTS: u64 = 33281;

/// Discord WebSocket gateway adapter.
pub struct DiscordAdapter {
    bot_token: Zeroizing<String>,
    client: reqwest::Client,
    allowed_guild_ids: Vec<String>,
    allowed_channel_ids: Vec<String>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    bot_user_id: Arc<RwLock<Option<String>>>,
    status: Arc<RwLock<ChannelStatus>>,
    /// Maps user platform_id -> last channel_id for send() fallback.
    user_to_channel: Arc<RwLock<HashMap<String, String>>>,
}

impl DiscordAdapter {
    pub fn new(
        bot_token: String,
        allowed_guild_ids: Vec<String>,
        allowed_channel_ids: Vec<String>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let client = reqwest::Client::builder()
            .default_headers(
                std::iter::once((
                    reqwest::header::CONTENT_TYPE,
                    reqwest::header::HeaderValue::from_static("application/json"),
                ))
                .collect(),
            )
            .build()
            .expect("reqwest client");

        Self {
            bot_token: Zeroizing::new(bot_token),
            client,
            allowed_guild_ids,
            allowed_channel_ids,
            shutdown_tx,
            shutdown_rx,
            bot_user_id: Arc::new(RwLock::new(None)),
            status: Arc::new(RwLock::new(ChannelStatus::default())),
            user_to_channel: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn build_auth_header(&self) -> String {
        format!("Bot {}", self.bot_token.as_str())
    }

}

#[async_trait]
impl ChannelAdapter for DiscordAdapter {
    fn name(&self) -> &str {
        "discord"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Discord
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let bot_token = self.bot_token.as_str().to_string();
        let client = self.client.clone();
        let allowed_guild_ids = self.allowed_guild_ids.clone();
        let allowed_channel_ids = self.allowed_channel_ids.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let bot_user_id = self.bot_user_id.clone();
        let status = self.status.clone();
        let user_to_channel = self.user_to_channel.clone();

        let stream = ReceiverStream::new(rx);

        tokio::spawn(async move {
            let mut backoff_ms = 1000u64;
            const MAX_BACKOFF_MS: u64 = 60_000;

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                match run_gateway_loop(
                    bot_token.clone(),
                    &client,
                    &allowed_guild_ids,
                    &allowed_channel_ids,
                    &mut shutdown_rx,
                    &tx,
                    &bot_user_id,
                    &status,
                    &user_to_channel,
                )
                .await
                {
                    Ok(()) => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                        backoff_ms = 1000;
                        warn!("Discord gateway disconnected, reconnecting...");
                    }
                    Err(e) => {
                        error!("Discord gateway error: {}", e);
                        status.write().await.last_error = Some(e.to_string());
                        status.write().await.connected = false;
                    }
                }

                if *shutdown_rx.borrow() {
                    break;
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
                backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
            }

            drop(tx);
        });

        Ok(Box::pin(stream))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let channel_id = {
            let map = self.user_to_channel.read().await;
            map.get(&user.platform_id).cloned()
        };

        let channel_id = match channel_id {
            Some(id) => id,
            None => {
                let url = format!("{}/users/@me/channels", DISCORD_API);
                let body = serde_json::json!({ "recipient_id": user.platform_id });
                let res = self
                    .client
                    .post(&url)
                    .header("Authorization", self.build_auth_header())
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| ChannelError::Send(e.to_string()))?;
                if !res.status().is_success() {
                    let status = res.status();
                    let text = res.text().await.unwrap_or_default();
                    return Err(ChannelError::Send(format!("{}: {}", status, text)));
                }
                let data: serde_json::Value = res.json().await?;
                data.get("id")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .ok_or_else(|| ChannelError::Send("No channel id in create DM response".into()))?
            }
        };

        self.send_to_channel(&channel_id, &content).await
    }

    async fn send_in_thread(
        &self,
        _user: &ChannelUser,
        content: ChannelContent,
        thread_id: &str,
    ) -> Result<(), ChannelError> {
        self.send_to_channel(thread_id, &content).await
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        // Return a snapshot; the real status is in Arc<RwLock>
        ChannelStatus::default()
    }
}

impl DiscordAdapter {
    async fn send_to_channel(
        &self,
        channel_id: &str,
        content: &ChannelContent,
    ) -> Result<(), ChannelError> {
        let text = match content {
            ChannelContent::Text(t) => t.clone(),
            ChannelContent::Image { url, caption } => {
                format!(
                    "{} {}",
                    url,
                    caption.as_deref().unwrap_or("")
                )
                .trim()
                .to_string()
            }
            ChannelContent::File { url, filename } => format!("[File: {}]({})", filename, url),
            _ => return Err(ChannelError::UnsupportedContent(format!("{:?}", content))),
        };

        let chunks = split_message(&text, DISCORD_MSG_LIMIT);
        for chunk in chunks {
            let body = serde_json::json!({ "content": chunk });
            let url = format!("{}/channels/{}/messages", DISCORD_API, channel_id);
            let res = self
                .client
                .post(&url)
                .header("Authorization", self.build_auth_header())
                .json(&body)
                .send()
                .await
                .map_err(|e| ChannelError::Send(e.to_string()))?;

            if res.status() == 429 {
                return Err(ChannelError::RateLimited);
            }
            if !res.status().is_success() {
                let status = res.status();
                let err_text = res.text().await.unwrap_or_default();
                return Err(ChannelError::Send(format!("{}: {}", status, err_text)));
            }
        }
        Ok(())
    }
}

async fn run_gateway_loop(
    bot_token: String,
    _client: &reqwest::Client,
    allowed_guild_ids: &[String],
    allowed_channel_ids: &[String],
    shutdown_rx: &mut watch::Receiver<bool>,
    tx: &mpsc::Sender<ChannelMessage>,
    bot_user_id: &Arc<RwLock<Option<String>>>,
    status: &Arc<RwLock<ChannelStatus>>,
    user_to_channel: &Arc<RwLock<HashMap<String, String>>>,
) -> Result<(), ChannelError> {
    let (ws_stream, _) = connect_async(DISCORD_GATEWAY)
        .await
        .map_err(|e| ChannelError::Connection(e.to_string()))?;

    let (mut write, mut read) = ws_stream.split();

    status.write().await.connected = true;
    status.write().await.started_at = Some(Utc::now());

    let mut last_seq: Option<u64> = None;
    let mut heartbeat_interval_ms: u64 = 41_250;
    let mut identified = false;
    let mut heartbeat_interval = tokio::time::interval(
        tokio::time::Duration::from_millis(heartbeat_interval_ms),
    );
    heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        let payload: serde_json::Value = serde_json::from_str(&text)
                            .map_err(|e| ChannelError::InvalidPayload(e.to_string()))?;
                        let op = payload.get("op").and_then(|v| v.as_u64()).unwrap_or(0);

                        match op {
                            10 => {
                                let d = payload.get("d").ok_or_else(|| ChannelError::InvalidPayload("HELLO missing d".into()))?;
                                heartbeat_interval_ms = d.get("heartbeat_interval").and_then(|v| v.as_u64()).unwrap_or(41250);
                                heartbeat_interval = tokio::time::interval(
                                    tokio::time::Duration::from_millis(heartbeat_interval_ms),
                                );
                                heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

                                let identify = serde_json::json!({
                                    "op": 2,
                                    "d": {
                                        "token": bot_token,
                                        "intents": DISCORD_INTENTS,
                                        "properties": {
                                            "os": "linux",
                                            "browser": "rune",
                                            "device": "rune"
                                        }
                                    }
                                });
                                let _ = write.send(WsMessage::Text(identify.to_string())).await;
                                identified = true;
                            }
                            11 => {}
                            0 => {
                                if let Some(s) = payload.get("s").and_then(|v| v.as_u64()) {
                                    last_seq = Some(s);
                                }
                                let t = payload.get("t").and_then(|v| v.as_str()).unwrap_or("");
                                if t == "READY" {
                                    if let Some(d) = payload.get("d") {
                                        if let Some(user) = d.get("user") {
                                            if let Some(id) = user.get("id").and_then(|v| v.as_str()) {
                                                bot_user_id.write().await.replace(id.to_string());
                                                info!("Discord bot user id: {}", id);
                                            }
                                        }
                                    }
                                } else if t == "MESSAGE_CREATE" {
                                    if let Some(data) = payload.get("d") {
                                        let bot_id = bot_user_id.read().await.clone();
                                        if let Some(msg) = parse_discord_message(
                                            data,
                                            bot_id.as_deref(),
                                            allowed_guild_ids,
                                            allowed_channel_ids,
                                        ) {
                                            let channel_id = data.get("channel_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                            let author_id = data.get("author").and_then(|a| a.get("id")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                                            if !channel_id.is_empty() && !author_id.is_empty() {
                                                user_to_channel.write().await.insert(author_id, channel_id);
                                            }
                                            let _ = tx.send(msg).await;
                                            status.write().await.messages_received += 1;
                                            status.write().await.last_message_at = Some(Utc::now());
                                        }
                                    }
                                }
                            }
                            7 => return Ok(()),
                            9 => return Ok(()),
                            _ => { debug!("Discord op {}", op); }
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) => return Ok(()),
                    Some(Err(e)) => return Err(ChannelError::Connection(e.to_string())),
                    None => return Ok(()),
                    _ => {}
                }
            }
            _ = heartbeat_interval.tick(), if identified => {
                let payload = serde_json::json!({"op": 1, "d": last_seq});
                if let Err(e) = write.send(WsMessage::Text(payload.to_string())).await {
                    return Err(ChannelError::Connection(e.to_string()));
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

/// Parse a Discord MESSAGE_CREATE event into a ChannelMessage.
pub fn parse_discord_message(
    data: &serde_json::Value,
    bot_user_id: Option<&str>,
    allowed_guild_ids: &[String],
    allowed_channel_ids: &[String],
) -> Option<ChannelMessage> {
    let author = data.get("author")?;
    let author_id = author.get("id")?.as_str()?;

    if bot_user_id == Some(author_id) {
        return None;
    }

    let channel_id = data.get("channel_id")?.as_str()?;
    let guild_id = data.get("guild_id").and_then(|v| v.as_str());

    let allowed = allowed_guild_ids.is_empty() && allowed_channel_ids.is_empty()
        || guild_id.is_some() && allowed_guild_ids.iter().any(|g| g == guild_id.unwrap())
        || allowed_channel_ids.iter().any(|c| c == channel_id);

    if !allowed {
        return None;
    }

    let content = data.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let msg_id = data.get("id")?.as_str()?;
    let timestamp = data
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let username = author
        .get("global_name")
        .or_else(|| author.get("username"))
        .and_then(|v| v.as_str())
        .unwrap_or(author_id);

    let is_group = guild_id.is_some();

    let mut metadata = HashMap::new();
    metadata.insert(
        "discord_channel_id".to_string(),
        serde_json::Value::String(channel_id.to_string()),
    );
    if let Some(g) = guild_id {
        metadata.insert(
            "discord_guild_id".to_string(),
            serde_json::Value::String(g.to_string()),
        );
    }

    Some(ChannelMessage {
        channel: ChannelType::Discord,
        platform_message_id: msg_id.to_string(),
        sender: ChannelUser {
            platform_id: author_id.to_string(),
            display_name: username.to_string(),
            rune_user: None,
        },
        content: ChannelContent::Text(content.to_string()),
        target_agent: None,
        timestamp,
        is_group,
        thread_id: Some(channel_id.to_string()),
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_discord_message_allowed() {
        let data = serde_json::json!({
            "id": "msg123",
            "channel_id": "ch1",
            "guild_id": "g1",
            "author": {
                "id": "user1",
                "username": "alice",
                "global_name": "Alice"
            },
            "content": "hello",
            "timestamp": "2024-01-15T12:00:00Z"
        });
        let msg = parse_discord_message(
            &data,
            Some("bot1"),
            &["g1".into()],
            &[],
        );
        assert!(msg.is_some());
        let m = msg.unwrap();
        assert_eq!(m.platform_message_id, "msg123");
        assert_eq!(m.sender.platform_id, "user1");
        assert_eq!(m.sender.display_name, "Alice");
        assert!(matches!(m.content, ChannelContent::Text(ref t) if t == "hello"));
        assert_eq!(m.thread_id, Some("ch1".into()));
    }

    #[test]
    fn test_parse_discord_message_ignores_bot() {
        let data = serde_json::json!({
            "id": "msg123",
            "channel_id": "ch1",
            "author": { "id": "bot1", "username": "bot" },
            "content": "hello"
        });
        let msg = parse_discord_message(&data, Some("bot1"), &[], &[]);
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_discord_message_filtered_guild() {
        let data = serde_json::json!({
            "id": "msg123",
            "channel_id": "ch1",
            "guild_id": "g_other",
            "author": { "id": "user1", "username": "alice" },
            "content": "hello"
        });
        let msg = parse_discord_message(&data, Some("bot1"), &["g1".into()], &[]);
        assert!(msg.is_none());
    }
}
