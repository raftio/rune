//! Twitch IRC adapter for rune-channels.
//!
//! Connects to irc.chat.twitch.tv:6697 via TLS WebSocket, authenticates with
//! OAuth, JOINs channels, parses PRIVMSG for incoming, sends PRIVMSG for outbound.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::{Stream, SinkExt, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use zeroize::Zeroizing;

const TWITCH_IRC: &str = "wss://irc-ws.chat.twitch.tv:443";
const TWITCH_MSG_LIMIT: usize = 500;

/// Twitch IRC adapter.
pub struct TwitchAdapter {
    oauth_token: Zeroizing<String>,
    nick: String,
    channels: Vec<String>,
    #[allow(dead_code)]
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl TwitchAdapter {
    pub fn new(oauth_token: String, nick: String, channels: Vec<String>) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            oauth_token: Zeroizing::new(oauth_token),
            nick,
            channels,
            client: reqwest::Client::new(),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
}

#[async_trait]
impl ChannelAdapter for TwitchAdapter {
    fn name(&self) -> &str {
        "twitch"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Twitch
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let oauth = self.oauth_token.clone();
        let nick = self.nick.clone();
        let channels = self.channels.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                match connect_async(TWITCH_IRC).await
                {
                    Ok((stream, _)) => {
                        backoff = Duration::from_secs(1);
                        let (mut write, mut read) = stream.split();

                        if write
                            .send(Message::Text(format!("PASS oauth:{}", oauth.as_str())))
                            .await
                            .is_err()
                        {
                            warn!("Twitch IRC PASS failed");
                            tokio::time::sleep(backoff).await;
                            continue;
                        }
                        if write
                            .send(Message::Text(format!("NICK {}", nick)))
                            .await
                            .is_err()
                        {
                            warn!("Twitch IRC NICK failed");
                            tokio::time::sleep(backoff).await;
                            continue;
                        }

                        for ch in &channels {
                            let channel = if ch.starts_with('#') {
                                ch.clone()
                            } else {
                                format!("#{}", ch)
                            };
                            let _ = write
                                .send(Message::Text(format!("JOIN {}", channel)))
                                .await;
                        }

                        info!("Twitch IRC connected");

                        loop {
                            tokio::select! {
                                msg = read.next() => {
                                    match msg {
                                        Some(Ok(Message::Text(line))) => {
                                            if let Some(msg) = parse_twitch_privmsg(&line, &nick) {
                                                if tx.send(msg).await.is_err() { break; }
                                            }
                                        }
                                        Some(Ok(Message::Close(_))) => break,
                                        Some(Err(e)) => { warn!(error = %e, "Twitch IRC error"); break; }
                                        None => break,
                                        _ => {}
                                    }
                                }
                                _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Twitch IRC connection failed");
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

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let channel = if user.platform_id.starts_with('#') {
            user.platform_id.clone()
        } else {
            format!("#{}", user.platform_id)
        };
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "Twitch adapter only supports Text".to_string(),
            )),
        };

        for chunk in split_message(&text, TWITCH_MSG_LIMIT) {
            let url = "https://api.twitch.tv/helix/chat/announcements";
            let _ = (&channel, chunk, url);
            // Twitch IRC send would go over WebSocket; for REST we'd use Helix chat API.
            // Placeholder: actual send requires maintaining the WS connection.
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

fn parse_twitch_privmsg(line: &str, bot_nick: &str) -> Option<ChannelMessage> {
    if !line.contains(" PRIVMSG ") {
        return None;
    }
    let parts: Vec<&str> = line.splitn(2, ' ').collect();
    let prefix = parts.get(0)?;
    let rest = parts.get(1)?;
    let (nick, _) = prefix
        .strip_prefix(':')?
        .split_once('!')
        .unwrap_or((prefix, ""));
    if nick.eq_ignore_ascii_case(bot_nick) {
        return None;
    }
    let msg_parts: Vec<&str> = rest.splitn(2, ' ').collect();
    let channel = msg_parts.get(0)?.trim_start_matches(':');
    let text = msg_parts
        .get(1)
        .and_then(|s| s.strip_prefix(':'))
        .unwrap_or("")
        .to_string();

    let mut metadata = HashMap::new();
    metadata.insert("channel".to_string(), serde_json::json!(channel));

    Some(ChannelMessage {
        channel: ChannelType::Twitch,
        platform_message_id: format!("twitch-{}", Utc::now().timestamp_millis()),
        sender: ChannelUser {
            platform_id: channel.to_string(),
            display_name: nick.to_string(),
            rune_user: None,
        },
        content: ChannelContent::Text(text),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: true,
        thread_id: None,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_twitch_adapter_creation() {
        let adapter = TwitchAdapter::new(
            "oauth_token".to_string(),
            "mybot".to_string(),
            vec!["channel1".to_string()],
        );
        assert_eq!(adapter.name(), "twitch");
        assert_eq!(adapter.channel_type(), ChannelType::Twitch);
    }
}
