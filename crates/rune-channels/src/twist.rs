//! Twist adapter for rune-channels.
//!
//! Polls Twist API for new messages. Sends via Twist API to post comments.

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
use std::time::Duration;
use tokio::sync::{mpsc, watch, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};
use zeroize::Zeroizing;

const TWIST_API: &str = "https://api.twist.com/api/v3";
const MAX_MSG_LEN: usize = 4000;
const POLL_INTERVAL_SECS: u64 = 10;

/// Twist adapter.
pub struct TwistAdapter {
    oauth_token: Zeroizing<String>,
    workspace_id: String,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    status: Arc<RwLock<ChannelStatus>>,
}

impl TwistAdapter {
    /// Create a new Twist adapter.
    pub fn new(oauth_token: String, workspace_id: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            oauth_token: Zeroizing::new(oauth_token),
            workspace_id,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            status: Arc::new(RwLock::new(ChannelStatus::default())),
        }
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.oauth_token.as_str())
    }
}

#[async_trait]
impl ChannelAdapter for TwistAdapter {
    fn name(&self) -> &str {
        "twist"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Twist
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let workspace_id = self.workspace_id.clone();
        let oauth_token = self.oauth_token.clone();
        let client = self.client.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let status = self.status.clone();

        tokio::spawn(async move {
            let mut last_updated: i64 = 0;
            let auth = format!("Bearer {}", oauth_token.as_str());

            status.write().await.connected = true;
            status.write().await.started_at = Some(Utc::now());
            info!("Twist polling started");

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
                    "{}/workspaces/{}/conversations?sort=updated&order=desc",
                    TWIST_API, workspace_id
                );

                match client
                    .get(&url)
                    .header("Authorization", &auth)
                    .header("Accept", "application/json")
                    .send()
                    .await
                {
                    Ok(resp) => {
                        if !resp.status().is_success() {
                            warn!(status = %resp.status(), "Twist conversations poll failed");
                            continue;
                        }

                        let json: serde_json::Value = match resp.json().await {
                            Ok(j) => j,
                            Err(e) => {
                                warn!(error = %e, "Twist parse failed");
                                continue;
                            }
                        };

                        let empty: Vec<serde_json::Value> = vec![];
                        let conversations = json
                            .get("data")
                            .and_then(|d| d.as_array())
                            .map(|a| a.as_slice())
                            .unwrap_or(empty.as_slice());

                        for conv in conversations {
                            let updated = conv
                                .get("updated")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);
                            if updated <= last_updated {
                                continue;
                            }
                            last_updated = last_updated.max(updated);

                            let conv_id = conv.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            let comments_url = format!(
                                "{}/comments/get?conversation_id={}",
                                TWIST_API, conv_id
                            );

                            if let Ok(comments_resp) = client
                                .get(&comments_url)
                                .header("Authorization", &auth)
                                .send()
                                .await
                            {
                                if comments_resp.status().is_success() {
                                    if let Ok(comments_json) = comments_resp.json::<serde_json::Value>().await {
                                        let empty_comments: Vec<serde_json::Value> = vec![];
                                        let comments = comments_json
                                            .get("data")
                                            .and_then(|d| d.as_array())
                                            .map(|a| a.as_slice())
                                            .unwrap_or(empty_comments.as_slice());
                                        for comment in comments {
                                            if let Some(msg) = parse_twist_comment(comment, conv_id) {
                                                if tx.send(msg).await.is_err() {
                                                    break;
                                                }
                                                status.write().await.messages_received += 1;
                                                status.write().await.last_message_at = Some(Utc::now());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Twist request failed");
                        status.write().await.last_error = Some(e.to_string());
                    }
                }
            }

            status.write().await.connected = false;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let thread_id = &user.platform_id;
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            ChannelContent::Image { url, caption } => {
                format!("{} {}", url, caption.as_deref().unwrap_or("")).trim().to_string()
            }
            _ => {
                return Err(ChannelError::UnsupportedContent(
                    "Twist adapter only supports Text and Image".to_string(),
                ))
            }
        };

        let url = format!("{}/comments/add", TWIST_API);
        let chunks = split_message(&text, MAX_MSG_LEN);

        for chunk in chunks {
            let body = serde_json::json!({
                "conversation_id": thread_id,
                "content": chunk,
            });

            let resp = self
                .client
                .post(&url)
                .header("Authorization", self.auth_header())
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

fn parse_twist_comment(comment: &serde_json::Value, conv_id: &str) -> Option<ChannelMessage> {
    let id = comment.get("id")?.as_i64()?.to_string();
    let content = comment.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let user = comment.get("user").and_then(|u| u.as_object());
    let user_id = user.and_then(|u| u.get("id")).and_then(|v| v.as_i64()).map(|i| i.to_string()).unwrap_or_default();
    let user_name = user
        .and_then(|u| u.get("name").or_else(|| u.get("username")))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut metadata = HashMap::new();
    metadata.insert("conversation_id".to_string(), serde_json::json!(conv_id));

    Some(ChannelMessage {
        channel: ChannelType::Twist,
        platform_message_id: id,
        sender: ChannelUser {
            platform_id: conv_id.to_string(),
            display_name: user_name.to_string(),
            rune_user: Some(user_id),
        },
        content: ChannelContent::Text(content.to_string()),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: true,
        thread_id: Some(conv_id.to_string()),
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_twist_adapter_creation() {
        let adapter = TwistAdapter::new(
            "test-token".to_string(),
            "workspace-123".to_string(),
        );
        assert_eq!(adapter.name(), "twist");
        assert_eq!(adapter.channel_type(), ChannelType::Twist);
    }
}
