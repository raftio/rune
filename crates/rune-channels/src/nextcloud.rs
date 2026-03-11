//! Nextcloud Talk adapter for rune-channels.
//!
//! Polls Nextcloud Talk API for new messages.
//! Sends via Talk API.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use base64::Engine;
use chrono::Utc;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};
use zeroize::Zeroizing;

const MAX_MSG_LEN: usize = 32000;
const POLL_INTERVAL_SECS: u64 = 5;

/// Nextcloud Talk adapter.
pub struct NextcloudAdapter {
    server_url: String,
    username: String,
    password: Zeroizing<String>,
    room_token: String,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    status: Arc<tokio::sync::RwLock<ChannelStatus>>,
}

impl NextcloudAdapter {
    /// Create a new Nextcloud Talk adapter.
    pub fn new(
        server_url: String,
        username: String,
        password: String,
        room_token: String,
    ) -> Self {
        let server = server_url.trim_end_matches('/').to_string();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            server_url: server,
            username,
            password: Zeroizing::new(password),
            room_token,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            status: Arc::new(tokio::sync::RwLock::new(ChannelStatus::default())),
        }
    }

    fn auth_header(&self) -> String {
        let credentials = format!("{}:{}", self.username, self.password.as_str());
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials.as_bytes());
        format!("Basic {}", encoded)
    }

    fn ocs_url(&self, path: &str) -> String {
        format!(
            "{}/ocs/v2.php/apps/spreed/api/v4{}",
            self.server_url, path
        )
    }
}

#[async_trait]
impl ChannelAdapter for NextcloudAdapter {
    fn name(&self) -> &str {
        "nextcloud"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Nextcloud
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let server_url = self.server_url.clone();
        let username = self.username.clone();
        let password = self.password.clone();
        let room_token = self.room_token.clone();
        let client = self.client.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let status = self.status.clone();

        tokio::spawn(async move {
            let mut last_known_message_id: i64 = 0;
            let auth = format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(
                    format!("{}:{}", username, password.as_str()).as_bytes()
                )
            );

            status.write().await.connected = true;
            status.write().await.started_at = Some(Utc::now());
            info!("Nextcloud Talk polling started");

            loop {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)) => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }

                if *shutdown_rx.borrow() {
                    break;
                }

                let url = format!(
                    "{}/ocs/v2.php/apps/spreed/api/v4/chat/{}",
                    server_url, room_token
                );

                match client
                    .get(&url)
                    .header("Authorization", &auth)
                    .header("OCS-APIRequest", "true")
                    .header("Accept", "application/json")
                    .send()
                    .await
                {
                    Ok(resp) => {
                        if !resp.status().is_success() {
                            warn!(status = %resp.status(), "Nextcloud Talk poll failed");
                            continue;
                        }

                        let json: serde_json::Value = match resp.json().await {
                            Ok(j) => j,
                            Err(e) => {
                                warn!(error = %e, "Nextcloud Talk parse failed");
                                continue;
                            }
                        };

                        let ocs = json.get("ocs").and_then(|o| o.get("data"));
                        let empty: Vec<serde_json::Value> = vec![];
                        let messages = ocs.and_then(|d| d.as_array()).map(|a| a.as_slice()).unwrap_or(empty.as_slice());

                        for msg in messages {
                            if let Some(parsed) = parse_nextcloud_message(msg, &room_token) {
                                let id: i64 = msg
                                    .get("id")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0);
                                if id > last_known_message_id {
                                    last_known_message_id = id;
                                    if tx.send(parsed).await.is_err() {
                                        break;
                                    }
                                    status.write().await.messages_received += 1;
                                    status.write().await.last_message_at = Some(Utc::now());
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Nextcloud Talk request failed");
                        status.write().await.last_error = Some(e.to_string());
                    }
                }
            }

            status.write().await.connected = false;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, _user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => {
                return Err(ChannelError::UnsupportedContent(
                    "Nextcloud adapter only supports Text".to_string(),
                ))
            }
        };

        let url = self.ocs_url(&format!("/chat/{}", self.room_token));
        let chunks = split_message(&text, MAX_MSG_LEN);

        for chunk in chunks {
            let body = serde_json::json!({
                "message": chunk,
            });

            let resp = self
                .client
                .post(&url)
                .header("Authorization", self.auth_header())
                .header("OCS-APIRequest", "true")
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| ChannelError::Send(e.to_string()))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_body = resp.text().await.unwrap_or_default();
                return Err(ChannelError::Send(format!("{}: {}", status, err_body)));
            }
        }

        self.status.write().await.messages_sent += 1;
        Ok(())
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        self.status.blocking_read().clone()
    }
}

fn parse_nextcloud_message(msg: &serde_json::Value, room_token: &str) -> Option<ChannelMessage> {
    let actor_type = msg.get("actorType")?.as_str()?;
    if actor_type == "bots" {
        return None;
    }

    let id = msg.get("id")?.as_i64()?.to_string();
    let message = msg.get("message")?.as_str()?;
    let actor_id = msg.get("actorId")?.as_str()?;
    let actor_display = msg.get("actorDisplayName").and_then(|v| v.as_str()).unwrap_or(actor_id);

    let mut metadata = HashMap::new();
    metadata.insert("room_token".to_string(), serde_json::json!(room_token));

    Some(ChannelMessage {
        channel: ChannelType::Nextcloud,
        platform_message_id: id,
        sender: ChannelUser {
            platform_id: room_token.to_string(),
            display_name: actor_display.to_string(),
            rune_user: Some(actor_id.to_string()),
        },
        content: ChannelContent::Text(message.to_string()),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: true,
        thread_id: Some(room_token.to_string()),
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nextcloud_adapter_creation() {
        let adapter = NextcloudAdapter::new(
            "https://cloud.example.com".to_string(),
            "user".to_string(),
            "secret".to_string(),
            "room123".to_string(),
        );
        assert_eq!(adapter.name(), "nextcloud");
        assert_eq!(adapter.channel_type(), ChannelType::Nextcloud);
    }
}
