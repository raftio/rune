//! Telegram Bot API adapter for rune-channels using long-polling.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser, LifecycleReaction,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, RwLock};
use tracing::{debug, info, warn};
use tokio_stream::wrappers::ReceiverStream;
use zeroize::Zeroizing;

const TELEGRAM_API: &str = "https://api.telegram.org";
const POLL_TIMEOUT: u64 = 30;
const MAX_MSG_LEN: usize = 4096;

/// Telegram Bot API adapter using long-polling.
pub struct TelegramAdapter {
    token: Zeroizing<String>,
    client: reqwest::Client,
    allowed_chat_ids: Vec<i64>,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    bot_user_id: Arc<RwLock<Option<i64>>>,
    status: Arc<RwLock<ChannelStatus>>,
}

impl TelegramAdapter {
    /// Create a new Telegram adapter.
    /// `allowed_chat_ids`: empty = allow all chats; non-empty = only these chat IDs.
    pub fn new(token: String, allowed_chat_ids: Vec<i64>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            token: Zeroizing::new(token),
            client,
            allowed_chat_ids,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            bot_user_id: Arc::new(RwLock::new(None)),
            status: Arc::new(RwLock::new(ChannelStatus::default())),
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", TELEGRAM_API, self.token.as_str(), method)
    }

    async fn get_me(&self) -> Result<i64, ChannelError> {
        let url = self.api_url("getMe");
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| ChannelError::Connection(e.to_string()))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ChannelError::InvalidPayload(e.to_string()))?;

        if !json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            let desc = json
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(ChannelError::Auth(desc.to_string()));
        }

        let result = json
            .get("result")
            .ok_or_else(|| ChannelError::InvalidPayload("getMe missing result".to_string()))?;
        let id = result
            .get("id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ChannelError::InvalidPayload("getMe result missing id".to_string()))?;

        Ok(id)
    }

    async fn api_send_message(
        &self,
        chat_id: i64,
        text: &str,
        parse_mode: Option<&str>,
    ) -> Result<(), ChannelError> {
        let chunks = split_message(text, MAX_MSG_LEN);
        let url = self.api_url("sendMessage");

        for chunk in chunks {
            let mut params = HashMap::new();
            params.insert("chat_id", chat_id.to_string());
            params.insert("text", chunk.to_string());
            if let Some(mode) = parse_mode {
                params.insert("parse_mode", mode.to_string());
            }

            let resp = self
                .client
                .post(&url)
                .json(&params)
                .send()
                .await
                .map_err(|e| ChannelError::Send(e.to_string()))?;

            let json: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ChannelError::InvalidPayload(e.to_string()))?;

            if !json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                let desc = json
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                return Err(ChannelError::Send(desc.to_string()));
            }
        }

        Ok(())
    }

    async fn api_send_chat_action(&self, chat_id: i64, action: &str) -> Result<(), ChannelError> {
        let url = self.api_url("sendChatAction");
        let params = serde_json::json!({
            "chat_id": chat_id,
            "action": action
        });

        let resp = self
            .client
            .post(&url)
            .json(&params)
            .send()
            .await
            .map_err(|e| ChannelError::Connection(e.to_string()))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ChannelError::InvalidPayload(e.to_string()))?;

        if !json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            let desc = json
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(ChannelError::Send(desc.to_string()));
        }

        Ok(())
    }

    async fn api_set_message_reaction(
        &self,
        chat_id: i64,
        message_id: i64,
        emoji: Option<&str>,
    ) -> Result<(), ChannelError> {
        let url = self.api_url("setMessageReaction");
        let reaction = emoji.map(|e| serde_json::json!([{"type": "emoji", "emoji": e}]));
        let params = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reaction": reaction
        });

        let resp = self
            .client
            .post(&url)
            .json(&params)
            .send()
            .await
            .map_err(|e| ChannelError::Send(e.to_string()))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ChannelError::InvalidPayload(e.to_string()))?;

        if !json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            let desc = json
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(ChannelError::Send(desc.to_string()));
        }

        Ok(())
    }

    #[allow(dead_code)]
    async fn get_file_url(&self, file_id: &str) -> Option<String> {
        Self::get_file_url_static(&self.client, self.token.as_str(), file_id).await
    }

    async fn get_file_url_static(
        client: &reqwest::Client,
        token: &str,
        file_id: &str,
    ) -> Option<String> {
        let url = format!("{}/bot{}/getFile", TELEGRAM_API, token);
        let params = serde_json::json!({"file_id": file_id});

        let resp = client.post(&url).json(&params).send().await.ok()?;
        let json: serde_json::Value = resp.json().await.ok()?;

        if !json.get("ok")?.as_bool().unwrap_or(false) {
            return None;
        }
        let path = json.get("result")?.get("file_path")?.as_str()?;

        Some(format!("{}/file/bot{}/{}", TELEGRAM_API, token, path))
    }
}

/// Extract file_ids from a Telegram message (photo, document, voice).
fn extract_file_ids(msg: &serde_json::Value) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(photos) = msg.get("photo").and_then(|v| v.as_array()) {
        if let Some(first) = photos.first() {
            if let Some(id) = first.get("file_id").and_then(|v| v.as_str()) {
                ids.push(id.to_string());
            }
        }
    }
    if let Some(doc) = msg.get("document") {
        if let Some(id) = doc.get("file_id").and_then(|v| v.as_str()) {
            ids.push(id.to_string());
        }
    }
    if let Some(voice) = msg.get("voice") {
        if let Some(id) = voice.get("file_id").and_then(|v| v.as_str()) {
            ids.push(id.to_string());
        }
    }
    ids
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn name(&self) -> &str {
        "telegram"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Telegram
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let bot_id = self.get_me().await?;
        {
            let mut bid = self.bot_user_id.write().await;
            *bid = Some(bot_id);
        }

        {
            let mut st = self.status.write().await;
            st.connected = true;
            st.started_at = Some(Utc::now());
        }

        info!(bot_id = %bot_id, "Telegram adapter started");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let client = self.client.clone();
        let token = self.token.clone();
        let allowed_chat_ids = self.allowed_chat_ids.clone();
        let status = self.status.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let mut offset: i64 = 0;
            let mut backoff = Duration::from_secs(1);
            const MAX_BACKOFF: Duration = Duration::from_secs(60);

            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            info!("Telegram adapter received shutdown signal");
                            break;
                        }
                    }
                    _ = tokio::time::sleep(Duration::ZERO) => {}
                }

                if *shutdown_rx.borrow() {
                    break;
                }

                let url = format!("{}/bot{}/getUpdates", TELEGRAM_API, token.as_str());
                let params = serde_json::json!({
                    "offset": offset,
                    "timeout": POLL_TIMEOUT,
                    "allowed_updates": ["message", "edited_message"]
                });

                match client.post(&url).json(&params).send().await {
                    Ok(resp) => {
                        backoff = Duration::from_secs(1);

                        let json: serde_json::Value = match resp.json().await {
                            Ok(j) => j,
                            Err(e) => {
                                warn!(error = %e, "Failed to parse getUpdates response");
                                continue;
                            }
                        };

                        if !json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                            let desc = json
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            warn!(description = %desc, "getUpdates failed");
                            {
                                let mut st = status.write().await;
                                st.last_error = Some(desc.to_string());
                                st.connected = false;
                            }
                            continue;
                        }

                        let updates: &[serde_json::Value] = json
                            .get("result")
                            .and_then(|v| v.as_array())
                            .map(|a| a.as_slice())
                            .unwrap_or(&[]);

                        for update in updates {
                            let update_id = update.get("update_id").and_then(|v| v.as_i64()).unwrap_or(0);
                            offset = update_id + 1;

                            // Resolve file URLs for photo/document/voice
                            let mut file_urls = HashMap::new();
                            if let Some(msg) = update.get("message").or_else(|| update.get("edited_message")) {
                                for file_id in extract_file_ids(msg) {
                                    if file_urls.contains_key(&file_id) {
                                        continue;
                                    }
                                    let url = Self::get_file_url_static(
                                        &client,
                                        token.as_str(),
                                        &file_id,
                                    )
                                    .await;
                                    if let Some(u) = url {
                                        file_urls.insert(file_id, u);
                                    }
                                }
                            }
                            if let Some(msg) =
                                parse_telegram_update(update, bot_id, &allowed_chat_ids, &file_urls)
                            {
                                if tx.send(msg).await.is_err() {
                                    break;
                                }
                                {
                                    let mut st = status.write().await;
                                    st.messages_received += 1;
                                    st.last_message_at = Some(Utc::now());
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "getUpdates network error");
                        {
                            let mut st = status.write().await;
                            st.last_error = Some(e.to_string());
                            st.connected = false;
                        }
                        tokio::time::sleep(backoff).await;
                        backoff = std::cmp::min(backoff * 2, MAX_BACKOFF);
                    }
                }
            }

            {
                let mut st = status.write().await;
                st.connected = false;
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let chat_id: i64 = user
            .platform_id
            .parse()
            .map_err(|_| ChannelError::Send("invalid chat_id".to_string()))?;

        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(chat_id, &text, Some("HTML"))
                    .await?;
                let mut st = self.status.write().await;
                st.messages_sent += 1;
                Ok(())
            }
            _ => Err(ChannelError::UnsupportedContent(
                "Telegram adapter only supports Text content for send".to_string(),
            )),
        }
    }

    async fn send_typing(&self, user: &ChannelUser) -> Result<(), ChannelError> {
        let chat_id: i64 = user
            .platform_id
            .parse()
            .map_err(|_| ChannelError::Send("invalid chat_id".to_string()))?;
        self.api_send_chat_action(chat_id, "typing").await
    }

    async fn send_reaction(
        &self,
        user: &ChannelUser,
        message_id: &str,
        reaction: &LifecycleReaction,
    ) -> Result<(), ChannelError> {
        let chat_id: i64 = user
            .platform_id
            .parse()
            .map_err(|_| ChannelError::Send("invalid chat_id".to_string()))?;
        let msg_id: i64 = message_id
            .parse()
            .map_err(|_| ChannelError::Send("invalid message_id".to_string()))?;
        let emoji = if reaction.emoji.is_empty() {
            None
        } else {
            Some(reaction.emoji.as_str())
        };
        self.api_set_message_reaction(chat_id, msg_id, emoji)
            .await
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        // block_in_place: status() is sync but RwLock::blocking_read blocks; must not block async runtime
        tokio::task::block_in_place(|| self.status.blocking_read().clone())
    }
}

/// Parse a Telegram Update JSON into a ChannelMessage.
/// Returns None if the update is not a supported message type or is filtered.
/// `file_urls` maps file_id -> download URL (from getFile); if absent, file_id is used as url.
pub fn parse_telegram_update(
    update: &serde_json::Value,
    _bot_id: i64,
    allowed_chat_ids: &[i64],
    file_urls: &HashMap<String, String>,
) -> Option<ChannelMessage> {
    let msg = update
        .get("message")
        .or_else(|| update.get("edited_message"))
        .and_then(|m| m.as_object())?;

    let chat = msg.get("chat")?.as_object()?;
    let chat_id = chat.get("id")?.as_i64()?;
    let is_group = matches!(
        chat.get("type")?.as_str(),
        Some("group") | Some("supergroup")
    );

    if !allowed_chat_ids.is_empty() && !allowed_chat_ids.contains(&chat_id) {
        debug!(chat_id = %chat_id, "Ignoring message from non-allowed chat");
        return None;
    }

    let from = msg.get("from").and_then(|f| f.as_object());
    let (display_name, from_id) = if let Some(f) = from {
        let id = f.get("id")?.as_i64()?;
        let first = f.get("first_name").and_then(|v| v.as_str()).unwrap_or("");
        let last = f.get("last_name").and_then(|v| v.as_str()).unwrap_or("");
        let name = if last.is_empty() {
            first.to_string()
        } else {
            format!("{} {}", first, last)
        };
        (name, Some(id))
    } else {
        ("Channel".to_string(), None)
    };
    // Use chat_id as platform_id so send() targets the correct chat (DM or group)
    let platform_id = chat_id.to_string();

    let message_id = msg
        .get("message_id")
        .and_then(|v| v.as_i64())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "0".to_string());

    let date_ts = msg.get("date").and_then(|v| v.as_i64()).unwrap_or(0);
    let timestamp = DateTime::from_timestamp(date_ts, 0).unwrap_or_else(Utc::now);

    let mut metadata = HashMap::new();
    if let Some(fid) = from_id {
        metadata.insert("from_id".to_string(), serde_json::json!(fid));
    }
    if let Some(thread_id) = msg.get("message_thread_id").and_then(|v| v.as_i64()) {
        metadata.insert(
            "thread_id".to_string(),
            serde_json::json!(thread_id.to_string()),
        );
    }

    let content = if let Some(text) = msg.get("text").and_then(|v| v.as_str()) {
        if text.starts_with('/') {
            let parts: Vec<&str> = text.splitn(2, ' ').collect();
            let name = parts[0].strip_prefix('/').unwrap_or("").to_string();
            let args: Vec<String> = if parts.len() > 1 {
                parts[1].split_whitespace().map(String::from).collect()
            } else {
                vec![]
            };
            ChannelContent::Command { name, args }
        } else {
            ChannelContent::Text(text.to_string())
        }
    } else if let Some(photos) = msg.get("photo").and_then(|v| v.as_array()) {
        let file_id = photos
            .first()
            .and_then(|p| p.get("file_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let url = file_urls
            .get(file_id)
            .cloned()
            .unwrap_or_else(|| file_id.to_string());
        let caption = msg
            .get("caption")
            .and_then(|v| v.as_str())
            .map(String::from);
        ChannelContent::Image {
            url,
            caption,
        }
    } else if let Some(doc) = msg.get("document").and_then(|v| v.as_object()) {
        let file_id = doc
            .get("file_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let url = file_urls
            .get(file_id)
            .cloned()
            .unwrap_or_else(|| file_id.to_string());
        let filename = doc
            .get("file_name")
            .and_then(|v| v.as_str())
            .unwrap_or("file")
            .to_string();
        ChannelContent::File {
            url,
            filename,
        }
    } else if let Some(voice) = msg.get("voice").and_then(|v| v.as_object()) {
        let file_id = voice
            .get("file_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let url = file_urls
            .get(file_id)
            .cloned()
            .unwrap_or_else(|| file_id.to_string());
        let duration = voice
            .get("duration")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        ChannelContent::Voice {
            url,
            duration_seconds: duration,
        }
    } else if let Some(loc) = msg.get("location").and_then(|v| v.as_object()) {
        let lat = loc.get("latitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let lon = loc.get("longitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
        ChannelContent::Location { lat, lon }
    } else if let Some(caption) = msg.get("caption").and_then(|v| v.as_str()) {
        ChannelContent::Text(caption.to_string())
    } else {
        return None;
    };

    let sender = ChannelUser {
        platform_id: platform_id.clone(),
        display_name,
        rune_user: from_id.map(|id| id.to_string()),
    };

    Some(ChannelMessage {
        channel: ChannelType::Telegram,
        platform_message_id: message_id,
        sender,
        content,
        target_agent: None,
        timestamp,
        is_group,
        thread_id: msg
            .get("message_thread_id")
            .and_then(|v| v.as_i64())
            .map(|v| v.to_string()),
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_telegram_update_text() {
        let update = serde_json::json!({
            "update_id": 123,
            "message": {
                "message_id": 456,
                "from": {"id": 789, "first_name": "Alice", "is_bot": false},
                "chat": {"id": 789, "type": "private"},
                "date": 1234567890,
                "text": "Hello world"
            }
        });
        let msg = parse_telegram_update(&update, 1, &[], &HashMap::new());
        assert!(msg.is_some());
        let m = msg.unwrap();
        assert_eq!(m.channel, ChannelType::Telegram);
        assert_eq!(m.platform_message_id, "456");
        assert_eq!(m.sender.platform_id, "789");
        assert_eq!(m.sender.display_name, "Alice");
        assert!(matches!(m.content, ChannelContent::Text(ref t) if t == "Hello world"));
    }

    #[test]
    fn test_parse_telegram_update_command() {
        let update = serde_json::json!({
            "update_id": 123,
            "message": {
                "message_id": 456,
                "from": {"id": 789, "first_name": "Bob", "is_bot": false},
                "chat": {"id": 789, "type": "private"},
                "date": 1234567890,
                "text": "/start help me"
            }
        });
        let msg = parse_telegram_update(&update, 1, &[], &HashMap::new());
        assert!(msg.is_some());
        let m = msg.unwrap();
        assert!(matches!(
            m.content,
            ChannelContent::Command {
                ref name,
                ref args
            } if name == "start" && args == &["help", "me"]
        ));
    }

    #[test]
    fn test_adapter_creation() {
        let adapter = TelegramAdapter::new("test:token".to_string(), vec![123, 456]);
        assert_eq!(adapter.name(), "telegram");
        assert_eq!(adapter.channel_type(), ChannelType::Telegram);
    }
}
