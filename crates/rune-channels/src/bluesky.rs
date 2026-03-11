//! Bluesky/AT Protocol adapter for rune-channels.
//!
//! Authenticates with PDS, polls notification feed for receiving.
//! send() uses com.atproto.repo.createRecord.

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
use tracing::{info, warn};
use zeroize::Zeroizing;

#[allow(dead_code)]
const MSG_LIMIT: usize = 300;

/// Bluesky/AT Protocol adapter.
pub struct BlueskyAdapter {
    handle: String,
    app_password: Zeroizing<String>,
    pds_url: String,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl BlueskyAdapter {
    pub fn new(handle: String, app_password: String, pds_url: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let pds = pds_url.trim_end_matches('/').to_string();
        Self {
            handle,
            app_password: Zeroizing::new(app_password),
            pds_url: pds,
            client: reqwest::Client::new(),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

    fn xrpc(&self, path: &str) -> String {
        format!("{}/xrpc/{}", self.pds_url, path.trim_start_matches('/'))
    }

    async fn create_session(&self) -> Result<(String, String), ChannelError> {
        let url = self.xrpc("com.atproto.server.createSession");
        let body = serde_json::json!({
            "identifier": self.handle,
            "password": self.app_password.as_str()
        });
        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(ChannelError::Auth(err));
        }
        let json: serde_json::Value = resp.json().await?;
        let token = json
            .get("accessJwt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::Auth("No accessJwt".into()))?;
        let did = json
            .get("did")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::Auth("No did".into()))?;
        Ok((token.to_string(), did.to_string()))
    }
}

#[async_trait]
impl ChannelAdapter for BlueskyAdapter {
    fn name(&self) -> &str {
        "bluesky"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Bluesky
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let client = self.client.clone();
        let handle = self.handle.clone();
        let app_password = self.app_password.clone();
        let pds_url = self.pds_url.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let mut token = None::<String>;
            let mut backoff = Duration::from_secs(1);

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                if token.is_none() {
                    match create_session_static(&client, &pds_url, &handle, app_password.as_str()).await {
                        Ok(t) => {
                            token = Some(t);
                            backoff = Duration::from_secs(1);
                        }
                        Err(e) => {
                            warn!(error = %e, "Bluesky session failed");
                            tokio::time::sleep(backoff).await;
                            backoff = std::cmp::min(backoff * 2, Duration::from_secs(60));
                            continue;
                        }
                    }
                }

                let t = token.as_ref().unwrap();
                let url = format!("{}/xrpc/app.bsky.notification.listNotifications", pds_url);
                match client
                    .get(&url)
                    .bearer_auth(t)
                    .send()
                    .await
                {
                    Ok(resp) => {
                        if let Ok(json) = resp.json::<serde_json::Value>().await {
                            if let Some(notifs) = json.get("notifications").and_then(|v| v.as_array()) {
                                for n in notifs {
                                    if let Some(msg) = parse_bluesky_notification(n) {
                                        if tx.send(msg).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Bluesky notification poll failed");
                        token = None;
                    }
                }

                tokio::select! {
                    _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
                    _ = tokio::time::sleep(Duration::from_secs(30)) => { }
                }
            }
            drop(tx);
        });

        info!("Bluesky adapter started");
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "Bluesky adapter only supports Text".to_string(),
            )),
        };

        let (token, did) = self.create_session().await?;
        let repo = did;
        let url = self.xrpc("com.atproto.repo.createRecord");
        let mut record = serde_json::json!({
            "repo": repo,
            "collection": "app.bsky.feed.post",
            "record": {
                "text": text,
                "createdAt": Utc::now().to_rfc3339()
            }
        });

        let parent = user.platform_id.as_str();
        if parent.starts_with("at://") {
            record["reply"] = serde_json::json!({
                "parent": { "uri": parent },
                "root": { "uri": parent }
            });
        }

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .json(&record)
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

async fn create_session_static(
    client: &reqwest::Client,
    pds_url: &str,
    handle: &str,
    password: &str,
) -> Result<String, ChannelError> {
    let url = format!("{}/xrpc/com.atproto.server.createSession", pds_url);
    let body = serde_json::json!({
        "identifier": handle,
        "password": password
    });
    let resp = client.post(&url).json(&body).send().await?;
    let json: serde_json::Value = resp.json().await?;
    let token = json
        .get("accessJwt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChannelError::Auth("No accessJwt".into()))?;
    Ok(token.to_string())
}

fn parse_bluesky_notification(json: &serde_json::Value) -> Option<ChannelMessage> {
    let uri = json.get("uri")?.as_str()?;
    let author = json.get("author")?;
    let did = author.get("did")?.as_str()?;
    let display = author.get("displayName").and_then(|v| v.as_str()).unwrap_or(did);
    let reason = json.get("reason")?.as_str().unwrap_or("");
    let record = json.get("record")?;
    let text = record.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let indexed_at = json.get("indexedAt").and_then(|v| v.as_str());
    let timestamp = indexed_at
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let mut metadata = HashMap::new();
    metadata.insert("reason".to_string(), serde_json::json!(reason));
    metadata.insert("uri".to_string(), serde_json::json!(uri));

    Some(ChannelMessage {
        channel: ChannelType::Bluesky,
        platform_message_id: uri.to_string(),
        sender: ChannelUser {
            platform_id: uri.to_string(),
            display_name: display.to_string(),
            rune_user: Some(did.to_string()),
        },
        content: ChannelContent::Text(text),
        target_agent: None,
        timestamp,
        is_group: false,
        thread_id: None,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bluesky_adapter_creation() {
        let adapter = BlueskyAdapter::new(
            "user.bsky.social".to_string(),
            "pass".to_string(),
            "https://bsky.social".to_string(),
        );
        assert_eq!(adapter.name(), "bluesky");
        assert_eq!(adapter.channel_type(), ChannelType::Bluesky);
    }
}
