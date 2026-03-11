//! Keybase adapter for rune-channels.
//!
//! Spawns `keybase chat api-listen` subprocess and parses JSON lines.
//! Sends via `keybase chat send` command.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};
use zeroize::Zeroizing;

const MAX_MSG_LEN: usize = 10000;

/// Keybase adapter.
pub struct KeybaseAdapter {
    username: String,
    paper_key: Zeroizing<String>,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl KeybaseAdapter {
    /// Create a new Keybase adapter.
    pub fn new(username: String, paper_key: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            username,
            paper_key: Zeroizing::new(paper_key),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
}

#[async_trait]
impl ChannelAdapter for KeybaseAdapter {
    fn name(&self) -> &str {
        "keybase"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Keybase
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let username = self.username.clone();
        let paper_key = self.paper_key.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let mut child = match Command::new("keybase")
                    .args([
                        "chat",
                        "api-listen",
                        "--no-banner",
                    ])
                    .env("KEYBASE_PAPER_KEY", paper_key.as_str())
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                {
                    Ok(c) => c,
                    Err(e) => {
                        warn!(error = %e, "keybase chat api-listen spawn failed");
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        continue;
                    }
                };

                let stdout = match child.stdout.take() {
                    Some(s) => s,
                    None => {
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        continue;
                    }
                };

                info!("Keybase api-listen started");
                let mut reader = BufReader::new(stdout);
                let mut line = String::new();

                loop {
                    tokio::select! {
                        result = reader.read_line(&mut line) => {
                            match result {
                                Ok(0) => break,
                                Ok(_) => {
                                    let trimmed = line.trim();
                                    if !trimmed.is_empty() {
                                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
                                            if let Some(msg) = parse_keybase_message(&value, &username) {
                                                if tx.send(msg).await.is_err() {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    line.clear();
                                }
                                Err(e) => {
                                    warn!(error = %e, "keybase read error");
                                    break;
                                }
                            }
                        }
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() {
                                let _ = child.kill().await;
                                break;
                            }
                        }
                    }
                }

                if *shutdown_rx.borrow() {
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => {
                return Err(ChannelError::UnsupportedContent(
                    "Keybase adapter only supports Text".to_string(),
                ))
            }
        };

        let conv_id = &user.platform_id;
        let chunks = split_message(&text, MAX_MSG_LEN);

        for chunk in chunks {
            let output = Command::new("keybase")
                .args(["chat", "send", conv_id, &chunk])
                .env("KEYBASE_PAPER_KEY", self.paper_key.as_str())
                .output()
                .await
                .map_err(|e| ChannelError::Io(e.into()))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(ChannelError::Send(stderr.to_string()));
            }
        }

        Ok(())
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        ChannelStatus {
            connected: true,
            ..ChannelStatus::default()
        }
    }
}

fn parse_keybase_message(value: &serde_json::Value, _bot_username: &str) -> Option<ChannelMessage> {
    let msg = value.get("msg")?.as_object()?;
    let msg_type = msg.get("type")?.as_i64()?;
    // type 1 = chat message, 2 = edit, etc.
    if msg_type != 1 {
        return None;
    }

    let content = msg.get("content")?.as_object()?;
    let text_obj = content.get("text")?.as_object()?;
    let body = text_obj.get("body")?.as_str()?;

    let conv = msg.get("conversation_id")?.get("typed")?.as_str()?;
    let sender = msg.get("sender")?.as_object()?;
    let username = sender.get("username")?.as_str()?;
    let device_name = sender
        .get("device_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let msg_id = msg.get("id")?.as_i64().map(|i| i.to_string()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let mut metadata = HashMap::new();
    metadata.insert("conversation_id".to_string(), serde_json::json!(conv));

    Some(ChannelMessage {
        channel: ChannelType::Keybase,
        platform_message_id: msg_id,
        sender: ChannelUser {
            platform_id: conv.to_string(),
            display_name: format!("{}{}", username, if device_name.is_empty() { "".to_string() } else { format!(":{}", device_name) }),
            rune_user: Some(username.to_string()),
        },
        content: ChannelContent::Text(body.to_string()),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: conv.contains(","),
        thread_id: Some(conv.to_string()),
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keybase_adapter_creation() {
        let adapter = KeybaseAdapter::new(
            "bot_user".to_string(),
            "paper key phrase".to_string(),
        );
        assert_eq!(adapter.name(), "keybase");
        assert_eq!(adapter.channel_type(), ChannelType::Keybase);
    }
}
