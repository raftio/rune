//! Reddit adapter for rune-channels.
//!
//! Polls Reddit API for new messages/mentions. send() posts reply via Reddit API.

use crate::error::ChannelError;
use crate::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType, ChannelUser,
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
use tracing::warn;
use zeroize::Zeroizing;

const REDDIT_API: &str = "https://oauth.reddit.com";
const REDDIT_TOKEN: &str = "https://www.reddit.com/api/v1/access_token";
#[allow(dead_code)]
const MSG_LIMIT: usize = 10000;

/// Reddit adapter.
pub struct RedditAdapter {
    client_id: String,
    client_secret: Zeroizing<String>,
    username: String,
    password: Zeroizing<String>,
    subreddits: Vec<String>,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl RedditAdapter {
    pub fn new(
        client_id: String,
        client_secret: String,
        username: String,
        password: String,
        subreddits: Vec<String>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let client = reqwest::Client::builder()
            .user_agent("rune-channels/1.0")
            .build()
            .expect("reqwest client");

        Self {
            client_id,
            client_secret: Zeroizing::new(client_secret),
            username,
            password: Zeroizing::new(password),
            subreddits,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

    async fn get_token(&self) -> Result<String, ChannelError> {
        let resp = self
            .client
            .post(REDDIT_TOKEN)
            .basic_auth(&self.client_id, Some(self.client_secret.as_str()))
            .form(&[
                ("grant_type", "password"),
                ("username", &self.username),
                ("password", self.password.as_str()),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(ChannelError::Auth(err));
        }

        let json: serde_json::Value = resp.json().await?;
        let token = json
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::Auth("No access_token".into()))?;
        Ok(token.to_string())
    }
}

#[async_trait]
impl ChannelAdapter for RedditAdapter {
    fn name(&self) -> &str {
        "reddit"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Reddit
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let client = self.client.clone();
        let client_id = self.client_id.clone();
        let client_secret = self.client_secret.clone();
        let username = self.username.clone();
        let password = self.password.clone();
        let _subreddits = self.subreddits.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let token = match get_token_static(&client, &client_id, client_secret.as_str(), &username, password.as_str()).await {
                    Ok(t) => t,
                    Err(e) => {
                        warn!(error = %e, "Reddit token failed");
                        tokio::time::sleep(backoff).await;
                        backoff = std::cmp::min(backoff * 2, Duration::from_secs(60));
                        continue;
                    }
                };

                let url = format!("{}/message/inbox", REDDIT_API);
                match client
                    .get(&url)
                    .bearer_auth(&token)
                    .send()
                    .await
                {
                    Ok(resp) => {
                        backoff = Duration::from_secs(1);
                        if let Ok(json) = resp.json::<serde_json::Value>().await {
                            if let Some(data) = json.get("data").and_then(|d| d.get("children")).and_then(|c| c.as_array()) {
                                for child in data {
                                    if let Some(msg) = child.get("data").and_then(|d| parse_reddit_message(d)) {
                                        if tx.send(msg).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Reddit API request failed");
                    }
                }

                tokio::select! {
                    _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
                    _ = tokio::time::sleep(Duration::from_secs(60)) => { }
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
                "Reddit adapter only supports Text".to_string(),
            )),
        };

        let token = self.get_token().await?;
        let parent_id = user.platform_id.as_str();
        let parent = if parent_id.starts_with("t3_") || parent_id.starts_with("t1_") {
            parent_id.to_string()
        } else {
            format!("t1_{}", parent_id)
        };

        let url = format!("{}/api/comment", REDDIT_API);
        let body = serde_json::json!({ "parent": parent, "text": text });
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(ChannelError::Send(err));
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

async fn get_token_static(
    client: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    username: &str,
    password: &str,
) -> Result<String, ChannelError> {
    let resp = client
        .post(REDDIT_TOKEN)
        .basic_auth(client_id, Some(client_secret))
        .form(&[
            ("grant_type", "password"),
            ("username", username),
            ("password", password),
        ])
        .send()
        .await?;
    let json: serde_json::Value = resp.json().await?;
    let token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChannelError::Auth("No access_token".into()))?;
    Ok(token.to_string())
}

fn parse_reddit_message(data: &serde_json::Value) -> Option<ChannelMessage> {
    let id = data.get("id")?.as_str()?;
    let author = data.get("author")?.as_str()?;
    let body = data.get("body")?.as_str()?.to_string();
    let created = data.get("created_utc").and_then(|v| v.as_f64()).unwrap_or(0.0) as i64;
    let timestamp = DateTime::from_timestamp(created, 0).unwrap_or_else(Utc::now);
    let subreddit = data.get("subreddit").and_then(|v| v.as_str()).unwrap_or("");

    let mut metadata = HashMap::new();
    metadata.insert("subreddit".to_string(), serde_json::json!(subreddit));

    Some(ChannelMessage {
        channel: ChannelType::Reddit,
        platform_message_id: id.to_string(),
        sender: ChannelUser {
            platform_id: id.to_string(),
            display_name: author.to_string(),
            rune_user: Some(author.to_string()),
        },
        content: ChannelContent::Text(body),
        target_agent: None,
        timestamp,
        is_group: true,
        thread_id: data.get("parent_id").and_then(|v| v.as_str()).map(String::from),
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reddit_adapter_creation() {
        let adapter = RedditAdapter::new(
            "client_id".to_string(),
            "secret".to_string(),
            "user".to_string(),
            "pass".to_string(),
            vec!["rust".to_string()],
        );
        assert_eq!(adapter.name(), "reddit");
        assert_eq!(adapter.channel_type(), ChannelType::Reddit);
    }
}
