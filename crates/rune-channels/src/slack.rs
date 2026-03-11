//! Slack Socket Mode adapter for rune-channels.
//!
//! Connects to Slack via Socket Mode (WebSocket), receives Events API events,
//! and sends messages via the Web API.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::{SinkExt, Stream, StreamExt};
use tokio_stream::wrappers::ReceiverStream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

const SLACK_API_BASE: &str = "https://slack.com/api";
const MAX_BACKOFF: Duration = Duration::from_secs(60);
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const SLACK_MSG_LIMIT: usize = 3000;

/// Reply context: where to send messages back to a user.
#[derive(Debug, Clone)]
struct ReplyContext {
    channel_id: String,
    thread_ts: Option<String>,
}

/// Slack Socket Mode adapter.
pub struct SlackAdapter {
    app_token: Zeroizing<String>,
    bot_token: Zeroizing<String>,
    client: reqwest::Client,
    allowed_channels: Vec<String>,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    bot_user_id: Arc<RwLock<Option<String>>>,
    /// Maps platform_user_id -> (channel_id, thread_ts) for replying.
    reply_context: Arc<RwLock<HashMap<String, ReplyContext>>>,
}

impl SlackAdapter {
    pub fn new(
        app_token: impl Into<String>,
        bot_token: impl Into<String>,
        allowed_channels: Vec<String>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            app_token: Zeroizing::new(app_token.into()),
            bot_token: Zeroizing::new(bot_token.into()),
            client: reqwest::Client::new(),
            allowed_channels,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            bot_user_id: Arc::new(RwLock::new(None)),
            reply_context: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Validate the bot token by calling auth.test.
    async fn validate_bot_token(&self) -> Result<String, ChannelError> {
        let url = format!("{SLACK_API_BASE}/auth.test");
        let res = self
            .client
            .post(&url)
            .bearer_auth(self.bot_token.as_str())
            .send()
            .await?;
        let status = res.status();
        let body: serde_json::Value = res.json().await?;
        if !status.is_success() {
            return Err(ChannelError::Auth(
                body["error"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string(),
            ));
        }
        let ok = body["ok"].as_bool().unwrap_or(false);
        if !ok {
            return Err(ChannelError::Auth(
                body["error"]
                    .as_str()
                    .unwrap_or("auth.test failed")
                    .to_string(),
            ));
        }
        let user_id = body["user_id"]
            .as_str()
            .ok_or_else(|| ChannelError::Auth("auth.test did not return user_id".into()))?
            .to_string();
        Ok(user_id)
    }

    /// Post a message via chat.postMessage, splitting long messages.
    async fn api_send_message(
        &self,
        channel_id: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<(), ChannelError> {
        let chunks = split_message(text, SLACK_MSG_LIMIT);
        for chunk in chunks {
            let mut payload = serde_json::json!({
                "channel": channel_id,
                "text": chunk,
            });
            if let Some(ts) = thread_ts {
                payload["thread_ts"] = serde_json::Value::String(ts.to_string());
            }
            let url = format!("{SLACK_API_BASE}/chat.postMessage");
            let res = self
                .client
                .post(&url)
                .bearer_auth(self.bot_token.as_str())
                .json(&payload)
                .send()
                .await?;
            let status = res.status();
            let body: serde_json::Value = res.json().await?;
            if !status.is_success() {
                return Err(ChannelError::Send(
                    body["error"]
                        .as_str()
                        .unwrap_or("chat.postMessage failed")
                        .to_string(),
                ));
            }
            if !body["ok"].as_bool().unwrap_or(false) {
                return Err(ChannelError::Send(
                    body["error"]
                        .as_str()
                        .unwrap_or("chat.postMessage failed")
                        .to_string(),
                ));
            }
        }
        Ok(())
    }
}

/// Obtain a WebSocket URL for Socket Mode via apps.connections.open.
async fn get_socket_mode_url(
    client: &reqwest::Client,
    app_token: &str,
) -> Result<String, ChannelError> {
    let url = format!("{SLACK_API_BASE}/apps.connections.open");
    let res = client
        .post(&url)
        .bearer_auth(app_token)
        .send()
        .await?;
    let status = res.status();
    let body: serde_json::Value = res.json().await?;
    if !status.is_success() {
        return Err(ChannelError::Connection(
            body["error"]
                .as_str()
                .unwrap_or("apps.connections.open failed")
                .to_string(),
        ));
    }
    let ok = body["ok"].as_bool().unwrap_or(false);
    if !ok {
        return Err(ChannelError::Connection(
            body["error"]
                .as_str()
                .unwrap_or("apps.connections.open failed")
                .to_string(),
        ));
    }
    let ws_url = body["url"]
        .as_str()
        .ok_or_else(|| ChannelError::Connection("apps.connections.open did not return url".into()))?
        .to_string();
    Ok(ws_url)
}

/// Parse a Slack Events API event into a ChannelMessage.
/// Returns None for events we should ignore (bot messages, filtered subtypes, etc.).
/// Parse a Slack message event (inner event object) into a ChannelMessage.
fn parse_slack_event(
    ev: &serde_json::Value,
    bot_user_id: Option<&str>,
    channel_id: &str,
    allowed_channels: &[String],
) -> Option<ChannelMessage> {
    let ev_type = ev.get("type")?.as_str()?;
    if ev_type != "message" {
        return None;
    }

    let subtype = ev.get("subtype").and_then(|v| v.as_str());

    // Filter bot messages (including our own).
    if subtype == Some("bot_message") {
        return None;
    }
    if ev.get("bot_id").is_some() {
        return None;
    }
    if let Some(uid) = ev.get("user").and_then(|v| v.as_str()) {
        if Some(uid) == bot_user_id {
            return None;
        }
    }

    // Filter by allowed_channels (empty = allow all).
    if !allowed_channels.is_empty() && !allowed_channels.iter().any(|c| c == channel_id) {
        return None;
    }

    // Handle message_changed: use the updated message.
    let (user_id, text, ts, thread_ts) = if subtype == Some("message_changed") {
        let msg = ev.get("message")?;
        let user_id = msg.get("user")?.as_str()?.to_string();
        let text = msg.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let ts = msg.get("ts")?.as_str()?.to_string();
        let thread_ts = msg.get("thread_ts").and_then(|v| v.as_str()).map(String::from);
        (user_id, text, ts, thread_ts)
    } else {
        // Regular message or other subtypes we might want (e.g. file_share).
        let user_id = ev.get("user")?.as_str()?.to_string();
        let text = ev.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let ts = ev.get("ts")?.as_str()?.to_string();
        let thread_ts = ev.get("thread_ts").and_then(|v| v.as_str()).map(String::from);
        (user_id, text, ts, thread_ts)
    };

    // Skip empty messages (e.g. some subtypes).
    if text.is_empty() && subtype != Some("file_share") {
        return None;
    }

    // Parse slash commands.
    let content = if text.starts_with('/') {
        let parts: Vec<&str> = text.splitn(2, ' ').collect();
        let name = parts[0][1..].to_string();
        let args: Vec<String> = if parts.len() > 1 {
            parts[1].split_whitespace().map(String::from).collect()
        } else {
            vec![]
        };
        ChannelContent::Command { name, args }
    } else {
        ChannelContent::Text(text)
    };

    let event_ts = ev
        .get("event_ts")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .map(|f| {
            let (secs, nsecs) = if f.fract() > 0.0 {
                (f as i64, ((f.fract() * 1e9) as u32))
            } else {
                (f as i64, 0)
            };
            DateTime::from_timestamp(secs, nsecs).unwrap_or_else(Utc::now)
        })
        .unwrap_or_else(Utc::now);

    let mut metadata = HashMap::new();
    metadata.insert(
        "slack_channel_id".to_string(),
        serde_json::Value::String(channel_id.to_string()),
    );
    if let Some(ref t) = thread_ts {
        metadata.insert(
            "slack_thread_ts".to_string(),
            serde_json::Value::String(t.clone()),
        );
    }

    // DMs (D) = not group; channels (C) and private groups (G) = group.
    let is_group = channel_id.starts_with('C') || channel_id.starts_with('G');

    Some(ChannelMessage {
        channel: ChannelType::Slack,
        platform_message_id: ts,
        sender: ChannelUser {
            platform_id: user_id,
            display_name: String::new(), // Slack API doesn't give display name in event; could fetch
            rune_user: None,
        },
        content,
        target_agent: None,
        timestamp: event_ts,
        is_group,
        thread_id: thread_ts,
        metadata,
    })
}

#[async_trait]
impl ChannelAdapter for SlackAdapter {
    fn name(&self) -> &str {
        "slack"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Slack
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let bot_id = self.validate_bot_token().await?;
        {
            let mut guard = self.bot_user_id.write().await;
            *guard = Some(bot_id.clone());
        }
        info!(bot_id = %bot_id, "Slack bot token validated");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let app_token = self.app_token.clone();
        let client = self.client.clone();
        let allowed_channels = self.allowed_channels.clone();
        let bot_user_id = self.bot_user_id.clone();
        let reply_context = self.reply_context.clone();
        let shutdown_rx = self.shutdown_rx.clone();

        let stream = ReceiverStream::new(rx);

        tokio::spawn(async move {
            let mut backoff = INITIAL_BACKOFF;
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let mut shutdown = shutdown_rx.clone();
                match run_socket_mode_loop(
                    &client,
                    app_token.as_str(),
                    &allowed_channels,
                    &bot_user_id,
                    &reply_context,
                    &mut shutdown,
                    &tx,
                )
                .await
                {
                    Ok(()) => {
                        backoff = INITIAL_BACKOFF;
                        if *shutdown_rx.borrow() {
                            break;
                        }
                        warn!("Slack Socket Mode disconnected, reconnecting...");
                    }
                    Err(e) => {
                        warn!(error = %e, backoff = ?backoff, "Slack Socket Mode error");
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

        Ok(Box::pin(stream))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let ctx = {
            let guard = self.reply_context.read().await;
            guard
                .get(&user.platform_id)
                .cloned()
                .ok_or_else(|| ChannelError::Send("No reply context for user".to_string()))?
        };

        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(
                    &ctx.channel_id,
                    &text,
                    ctx.thread_ts.as_deref(),
                )
                .await
            }
            _ => Err(ChannelError::UnsupportedContent(format!(
                "Slack adapter does not support {:?}",
                content
            ))),
        }
    }

    async fn send_in_thread(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
        thread_id: &str,
    ) -> Result<(), ChannelError> {
        let ctx = {
            let guard = self.reply_context.read().await;
            guard
                .get(&user.platform_id)
                .cloned()
                .ok_or_else(|| ChannelError::Send("No reply context for user".to_string()))?
        };

        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(
                    &ctx.channel_id,
                    &text,
                    Some(thread_id),
                )
                .await
            }
            _ => Err(ChannelError::UnsupportedContent(format!(
                "Slack adapter does not support {:?}",
                content
            ))),
        }
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        ChannelStatus::default()
    }
}

async fn run_socket_mode_loop(
    client: &reqwest::Client,
    app_token: &str,
    allowed_channels: &[String],
    bot_user_id: &Arc<RwLock<Option<String>>>,
    reply_context: &Arc<RwLock<HashMap<String, ReplyContext>>>,
    shutdown_rx: &mut watch::Receiver<bool>,
    tx: &mpsc::Sender<ChannelMessage>,
) -> Result<(), ChannelError> {
    let ws_url = get_socket_mode_url(client, app_token).await?;
    info!(url = %ws_url, "Connecting to Slack Socket Mode");
    let (stream, _) = connect_async(&ws_url)
        .await
        .map_err(|e| ChannelError::Connection(e.to_string()))?;
    let (mut write, mut read) = stream.split();
    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(e) = handle_socket_envelope(
                            &text, allowed_channels, bot_user_id, reply_context, &mut write, tx,
                        ).await {
                            warn!(error = %e, "Error handling Socket Mode envelope");
                        }
                    }
                    Some(Ok(Message::Close(_))) => { info!("Slack WebSocket closed"); return Ok(()); }
                    Some(Err(e)) => return Err(ChannelError::Connection(e.to_string())),
                    None => return Ok(()),
                    _ => {}
                }
            }
            _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { return Ok(()); } }
        }
    }
}

async fn handle_socket_envelope(
    text: &str,
    allowed_channels: &[String],
    bot_user_id: &Arc<RwLock<Option<String>>>,
    reply_context: &Arc<RwLock<HashMap<String, ReplyContext>>>,
    write: &mut futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
        Message,
    >,
    tx: &mpsc::Sender<ChannelMessage>,
) -> Result<(), ChannelError> {
    let envelope: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| ChannelError::InvalidPayload(e.to_string()))?;

    let envelope_id = envelope["envelope_id"].as_str();
    let msg_type = envelope["type"].as_str().unwrap_or("");

    // Ack immediately for events_api.
    if msg_type == "events_api" {
        if let Some(eid) = envelope_id {
            let ack = serde_json::json!({ "envelope_id": eid });
            write
                .send(Message::Text(ack.to_string()))
                .await
                .map_err(|e| ChannelError::Connection(e.to_string()))?;
        }

        let payload = envelope
            .get("payload")
            .ok_or_else(|| ChannelError::InvalidPayload("events_api missing payload".into()))?;

        if payload["type"].as_str() != Some("event_callback") {
            return Ok(());
        }

        let event = payload
            .get("event")
            .ok_or_else(|| ChannelError::InvalidPayload("event_callback missing event".into()))?;

        let channel_id = event["channel"].as_str().unwrap_or("");

        let bot_id = bot_user_id.read().await.clone();
        if let Some(msg) = parse_slack_event(event, bot_id.as_deref(), channel_id, allowed_channels)
        {
            {
                let mut guard = reply_context.write().await;
                guard.insert(
                    msg.sender.platform_id.clone(),
                    ReplyContext {
                        channel_id: channel_id.to_string(),
                        thread_ts: msg.thread_id.clone(),
                    },
                );
            }
            let _ = tx.send(msg).await;
            debug!("Forwarded Slack message to channel stream");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChannelContent;

    #[test]
    fn test_parse_slack_event_basic() {
        let event = serde_json::json!({
            "type": "message",
            "channel": "C123ABC",
            "user": "U123ABC",
            "text": "Hello world",
            "ts": "1355517523.000005",
            "event_ts": "1355517523.000005"
        });
        let msg = parse_slack_event(&event, Some("BOT123"), "C123ABC", &[]);
        assert!(msg.is_some());
        let m = msg.unwrap();
        assert_eq!(m.platform_message_id, "1355517523.000005");
        assert_eq!(m.sender.platform_id, "U123ABC");
        assert!(matches!(m.content, ChannelContent::Text(ref t) if t == "Hello world"));
        assert_eq!(m.channel, ChannelType::Slack);
    }

    #[test]
    fn test_parse_slack_event_filters_bot() {
        let event = serde_json::json!({
            "type": "message",
            "subtype": "bot_message",
            "channel": "C123ABC",
            "bot_id": "B123",
            "text": "Bot says hi",
            "ts": "1355517523.000005"
        });
        let msg = parse_slack_event(&event, Some("BOT123"), "C123ABC", &[]);
        assert!(msg.is_none());

        let event_user_bot = serde_json::json!({
            "type": "message",
            "channel": "C123ABC",
            "user": "BOT123",
            "text": "Our bot",
            "ts": "1355517523.000006"
        });
        let msg2 = parse_slack_event(&event_user_bot, Some("BOT123"), "C123ABC", &[]);
        assert!(msg2.is_none());
    }

    #[test]
    fn test_parse_slack_event_command() {
        let event = serde_json::json!({
            "type": "message",
            "channel": "C123",
            "user": "U123",
            "text": "/help foo bar",
            "ts": "1.0",
            "event_ts": "1.0"
        });
        let msg = parse_slack_event(&event, Some("BOT"), "C123", &[]);
        assert!(msg.is_some());
        let m = msg.unwrap();
        assert!(matches!(m.content, ChannelContent::Command { name, args } if name == "help" && args == ["foo", "bar"]));
    }

    #[test]
    fn test_slack_adapter_creation() {
        let adapter = SlackAdapter::new(
            "xapp-1-token",
            "xoxb-bot-token",
            vec!["C123".to_string()],
        );
        assert_eq!(adapter.name(), "slack");
        assert_eq!(adapter.channel_type(), ChannelType::Slack);
    }
}
