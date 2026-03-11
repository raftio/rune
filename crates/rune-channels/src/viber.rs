//! Viber adapter for rune-channels.
//!
//! Spawns HTTP server for webhook events, registers webhook via set_webhook API.
//! send() uses send_message API.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use axum::{body::Bytes, http::StatusCode, response::IntoResponse, routing::post, Router};
use chrono::Utc;
use futures::Stream;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tracing::info;
use zeroize::Zeroizing;

const VIBER_API: &str = "https://chatapi.viber.com/pa";
const MSG_LIMIT: usize = 7000;

/// Viber adapter.
pub struct ViberAdapter {
    auth_token: Zeroizing<String>,
    webhook_url: String,
    webhook_port: u16,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl ViberAdapter {
    pub fn new(auth_token: String, webhook_url: String, webhook_port: u16) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            auth_token: Zeroizing::new(auth_token),
            webhook_url,
            webhook_port,
            client: reqwest::Client::new(),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
}

#[async_trait]
impl ChannelAdapter for ViberAdapter {
    fn name(&self) -> &str {
        "viber"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Viber
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let webhook = format!("{}/webhook", self.webhook_url.trim_end_matches('/'));
        let set_url = format!("{}/set_webhook", VIBER_API);
        let resp = self
            .client
            .post(&set_url)
            .json(&serde_json::json!({
                "url": webhook,
                "event_types": ["delivered", "seen", "failed", "subscribed", "unsubscribed", "conversation_started", "message"]
            }))
            .header("X-Viber-Auth-Token", self.auth_token.as_str())
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(ChannelError::Config(err));
        }

        info!(webhook = %webhook, "Viber webhook registered");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let tx_clone = tx.clone();
        let port = self.webhook_port;
        let mut shutdown_rx = self.shutdown_rx.clone();

        let app = Router::new().route(
            "/webhook",
            post(move |body: Bytes| {
                let tx = tx_clone.clone();
                async move {
                    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(body.as_ref()) {
                        let event = json.get("event").and_then(|v| v.as_str()).unwrap_or("");
                        if event == "message" {
                            if let Some(msg) = json.get("message").and_then(|m| parse_viber_message(m, &json)) {
                                let _ = tx.send(msg).await;
                            }
                        }
                    }
                    (StatusCode::OK, "OK").into_response()
                }
            }),
        );

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        info!(port = %port, "Viber webhook server starting");
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let server = axum::serve(listener, app);

        tokio::spawn(async move {
            tokio::select! {
                _ = server => {}
                _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { drop(tx); } }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let receiver = &user.platform_id;
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "Viber adapter only supports Text".to_string(),
            )),
        };

        let url = format!("{}/send_message", VIBER_API);
        for chunk in split_message(&text, MSG_LIMIT) {
            let body = serde_json::json!({
                "receiver": receiver,
                "type": "text",
                "text": chunk
            });
            let resp = self
                .client
                .post(&url)
                .header("X-Viber-Auth-Token", self.auth_token.as_str())
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let err = resp.text().await.unwrap_or_default();
                return Err(ChannelError::Send(err));
            }
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

fn parse_viber_message(msg: &serde_json::Value, ev: &serde_json::Value) -> Option<ChannelMessage> {
    let sender = ev.get("sender")?.get("id")?.as_str()?;
    let name = ev.get("sender")?.get("name").and_then(|v| v.as_str()).unwrap_or(sender);
    let msg_type = msg.get("type")?.as_str()?;
    let text = if msg_type == "text" {
        msg.get("text")?.as_str()?.to_string()
    } else {
        format!("[{}]", msg_type)
    };
    let token = msg.get("token")?.as_str().unwrap_or("0");

    Some(ChannelMessage {
        channel: ChannelType::Viber,
        platform_message_id: token.to_string(),
        sender: ChannelUser {
            platform_id: sender.to_string(),
            display_name: name.to_string(),
            rune_user: None,
        },
        content: ChannelContent::Text(text),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: false,
        thread_id: None,
        metadata: HashMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_viber_adapter_creation() {
        let adapter = ViberAdapter::new(
            "token".to_string(),
            "https://example.com".to_string(),
            8080,
        );
        assert_eq!(adapter.name(), "viber");
        assert_eq!(adapter.channel_type(), ChannelType::Viber);
    }
}
