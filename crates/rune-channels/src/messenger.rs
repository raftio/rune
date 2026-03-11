//! Facebook Messenger adapter for rune-channels.
//!
//! Spawns HTTP server for webhook verification and message events.
//! send() uses the Send API.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use axum::{body::Bytes, extract::Query, http::StatusCode, response::IntoResponse, routing::get, routing::post, Router};
use chrono::Utc;
use futures::Stream;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::collections::HashMap as Map;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tracing::info;
use zeroize::Zeroizing;

type HmacSha256 = Hmac<Sha256>;
const MESSENGER_API: &str = "https://graph.facebook.com/v18.0";
const MSG_LIMIT: usize = 2000;

/// Facebook Messenger adapter.
pub struct MessengerAdapter {
    page_access_token: Zeroizing<String>,
    verify_token: String,
    app_secret: Zeroizing<String>,
    webhook_port: u16,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl MessengerAdapter {
    pub fn new(
        page_access_token: String,
        verify_token: String,
        app_secret: String,
        webhook_port: u16,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            page_access_token: Zeroizing::new(page_access_token),
            verify_token,
            app_secret: Zeroizing::new(app_secret),
            webhook_port,
            client: reqwest::Client::new(),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
}

#[async_trait]
impl ChannelAdapter for MessengerAdapter {
    fn name(&self) -> &str {
        "messenger"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Messenger
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let tx_clone = tx.clone();
        let verify_token = self.verify_token.clone();
        let app_secret = self.app_secret.clone();
        let port = self.webhook_port;
        let mut shutdown_rx = self.shutdown_rx.clone();

        let app = Router::new()
            .route(
                "/webhook",
                get(move |Query(params): Query<Map<String, String>>| async move {
                    let mode = params.get("hub.mode").map(|s| s.as_str());
                    let token = params.get("hub.verify_token").map(|s| s.as_str());
                    let challenge = params.get("hub.challenge").cloned().unwrap_or_default();
                    if mode == Some("subscribe") && token == Some(verify_token.as_str()) {
                        (StatusCode::OK, challenge).into_response()
                    } else {
                        (StatusCode::FORBIDDEN, "Forbidden").into_response()
                    }
                }),
            )
            .route(
                "/webhook",
                post(move |sig: axum::http::HeaderMap, body: Bytes| {
                    let tx = tx_clone.clone();
                    let app_secret = app_secret.clone();
                    async move {
                        let sig_val = sig
                            .get("x-hub-signature-256")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("");
                        let body_bytes = body.as_ref();
                        if !sig_val.is_empty() {
                            let expected = {
                                let mut mac = HmacSha256::new_from_slice(app_secret.as_bytes()).expect("HMAC");
                                mac.update(body_bytes);
                                hex::encode(mac.finalize().into_bytes())
                            };
                            let got = sig_val.strip_prefix("sha256=").unwrap_or(sig_val);
                            if got != expected {
                                return (StatusCode::UNAUTHORIZED, "Invalid signature").into_response();
                            }
                        }
                        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(body_bytes) {
                            if let Some(entry) = json.get("entry").and_then(|v| v.as_array()) {
                                for e in entry {
                                    if let Some(messaging) = e.get("messaging").and_then(|v| v.as_array()) {
                                        for m in messaging {
                                            if m.get("message").is_some() {
                                                if let Some(msg) = parse_messenger_event(m) {
                                                    let _ = tx.send(msg).await;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        (StatusCode::OK, "OK").into_response()
                    }
                }),
            );

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        info!(port = %port, "Messenger webhook server starting");
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
        let recipient_id = &user.platform_id;
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "Messenger adapter only supports Text".to_string(),
            )),
        };

        let url = format!("{}/me/messages", MESSENGER_API);
        for chunk in split_message(&text, MSG_LIMIT) {
            let body = serde_json::json!({
                "recipient": { "id": recipient_id },
                "message": { "text": chunk }
            });
            let resp = self
                .client
                .post(&url)
                .query(&[("access_token", self.page_access_token.as_str())])
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

fn parse_messenger_event(ev: &serde_json::Value) -> Option<ChannelMessage> {
    let sender = ev.get("sender")?.get("id")?.as_str()?;
    let msg = ev.get("message")?;
    let text = msg.get("text")?.as_str()?.to_string();
    let mid = msg.get("mid")?.as_str()?;

    Some(ChannelMessage {
        channel: ChannelType::Messenger,
        platform_message_id: mid.to_string(),
        sender: ChannelUser {
            platform_id: sender.to_string(),
            display_name: sender.to_string(),
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
    fn test_messenger_adapter_creation() {
        let adapter = MessengerAdapter::new(
            "token".to_string(),
            "verify".to_string(),
            "secret".to_string(),
            8080,
        );
        assert_eq!(adapter.name(), "messenger");
        assert_eq!(adapter.channel_type(), ChannelType::Messenger);
    }
}
