//! WhatsApp Business API adapter.
//!
//! Receives webhook events (verification GET and message POST) and sends
//! messages via the WhatsApp Cloud API.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};
use zeroize::Zeroizing;

const WHATSAPP_API: &str = "https://graph.facebook.com/v18.0";
const MAX_MSG_LEN: usize = 4096;

/// WhatsApp Business API adapter.
pub struct WhatsAppAdapter {
    phone_number_id: String,
    access_token: Zeroizing<String>,
    verify_token: String,
    webhook_port: u16,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    status: Arc<RwLock<ChannelStatus>>,
}

impl WhatsAppAdapter {
    /// Create a new WhatsApp adapter.
    pub fn new(
        phone_number_id: String,
        access_token: String,
        verify_token: String,
        webhook_port: u16,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("reqwest client");

        Self {
            phone_number_id,
            access_token: Zeroizing::new(access_token),
            verify_token,
            webhook_port,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            status: Arc::new(RwLock::new(ChannelStatus::default())),
        }
    }

    async fn api_send_message(&self, to: &str, text: &str) -> Result<(), ChannelError> {
        let chunks = split_message(text, MAX_MSG_LEN);
        let url = format!("{}/{}/messages", WHATSAPP_API, self.phone_number_id);

        for chunk in chunks {
            let body = serde_json::json!({
                "messaging_product": "whatsapp",
                "recipient_type": "individual",
                "to": to.trim_start_matches('+'),
                "type": "text",
                "text": { "body": chunk }
            });

            let resp = self
                .client
                .post(&url)
                .bearer_auth(self.access_token.as_str())
                .json(&body)
                .send()
                .await
                .map_err(|e| ChannelError::Send(e.to_string()))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(ChannelError::Send(format!("{}: {}", status, text)));
            }
        }

        Ok(())
    }
}

/// Parse a WhatsApp webhook entry into ChannelMessage(s).
fn parse_whatsapp_webhook(entry: &serde_json::Value) -> Vec<ChannelMessage> {
    let mut messages = Vec::new();
    let Some(changes) = entry.get("changes").and_then(|c| c.as_array()) else {
        return messages;
    };
    for change in changes {
        let Some(value) = change.get("value").and_then(|v| v.as_object()) else {
            continue;
        };
        let Some(messages_arr) = value.get("messages").and_then(|m| m.as_array()) else {
            continue;
        };
        for msg in messages_arr {
            let Some(from) = msg.get("from").and_then(|f| f.as_str()).map(String::from) else {
                continue;
            };
            let Some(id) = msg.get("id").and_then(|i| i.as_str()).map(String::from) else {
                continue;
            };
            let timestamp = msg
                .get("timestamp")
                .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok())))
                .and_then(|ts| DateTime::from_timestamp(ts, 0))
                .unwrap_or_else(Utc::now);

            let contacts = value.get("contacts").and_then(|c| c.as_array());
            let display_name = contacts
                .and_then(|c| c.first())
                .and_then(|c| c.get("profile").and_then(|p| p.get("name")).and_then(|n| n.as_str()))
                .unwrap_or(&from)
                .to_string();

            let content = if let Some(text_obj) = msg.get("text") {
                let Some(body) = text_obj.get("body").and_then(|b| b.as_str()).map(String::from) else {
                    continue;
                };
                if body.starts_with('/') {
                    let parts: Vec<&str> = body.splitn(2, ' ').collect();
                    let name = parts[0].strip_prefix('/').unwrap_or("").to_string();
                    let args: Vec<String> = if parts.len() > 1 {
                        parts[1].split_whitespace().map(String::from).collect()
                    } else {
                        vec![]
                    };
                    ChannelContent::Command { name, args }
                } else {
                    ChannelContent::Text(body)
                }
            } else if let Some(img) = msg.get("image") {
                let Some(id) = img.get("id").and_then(|i| i.as_str()).map(String::from) else {
                    continue;
                };
                let caption = img.get("caption").and_then(|v| v.as_str()).map(String::from);
                ChannelContent::Image {
                    url: format!("https://graph.facebook.com/v18.0/{}", id),
                    caption,
                }
            } else {
                continue;
            };

            let mut metadata = HashMap::new();
            metadata.insert("wa_msg_id".to_string(), serde_json::json!(id));

            messages.push(ChannelMessage {
                channel: ChannelType::WhatsApp,
                platform_message_id: id,
                sender: ChannelUser {
                    platform_id: from,
                    display_name,
                    rune_user: None,
                },
                content,
                target_agent: None,
                timestamp,
                is_group: false,
                thread_id: msg.get("context").and_then(|c| c.get("id")).and_then(|i| i.as_str()).map(String::from),
                metadata,
            });
        }
    }
    messages
}

#[async_trait]
impl ChannelAdapter for WhatsAppAdapter {
    fn name(&self) -> &str {
        "whatsapp"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::WhatsApp
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let _access_token = self.access_token.clone();
        let _phone_number_id = self.phone_number_id.clone();
        let verify_token = self.verify_token.clone();
        let webhook_port = self.webhook_port;
        let _client = self.client.clone();
        let status = self.status.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        let app = axum::Router::new()
            .route(
                "/webhook",
                axum::routing::get(whatsapp_verify).post(whatsapp_webhook),
            )
            .with_state(Arc::new(WhatsAppWebhookState {
                tx,
                verify_token,
                status,
            }));

        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], webhook_port));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| ChannelError::Config(e.to_string()))?;

        let server = axum::serve(listener, app);

        tokio::spawn(async move {
            tokio::select! {
                _ = server => {}
                _ = async {
                    loop {
                        let _ = shutdown_rx.changed().await;
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                } => {}
            }
        });

        {
            let mut st = self.status.write().await;
            st.connected = true;
            st.started_at = Some(Utc::now());
        }
        info!(port = webhook_port, "WhatsApp adapter started");

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(&user.platform_id, &text).await?;
                let mut st = self.status.write().await;
                st.messages_sent += 1;
                Ok(())
            }
            _ => Err(ChannelError::UnsupportedContent(
                "WhatsApp adapter only supports Text content".into(),
            )),
        }
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        self.status.blocking_read().clone()
    }
}

struct WhatsAppWebhookState {
    tx: mpsc::Sender<ChannelMessage>,
    verify_token: String,
    status: Arc<RwLock<ChannelStatus>>,
}

async fn whatsapp_verify(
    axum::extract::State(state): axum::extract::State<Arc<WhatsAppWebhookState>>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl axum::response::IntoResponse {
    let mode = params.get("hub.mode").map(String::as_str);
    let token = params.get("hub.verify_token").map(String::as_str);
    let challenge = params.get("hub.challenge").cloned().unwrap_or_default();

    if mode == Some("subscribe") && token.as_deref() == Some(state.verify_token.as_str()) {
        (axum::http::StatusCode::OK, challenge)
    } else {
        (axum::http::StatusCode::FORBIDDEN, String::new())
    }
}

async fn whatsapp_webhook(
    axum::extract::State(state): axum::extract::State<Arc<WhatsAppWebhookState>>,
    body: String,
) -> impl axum::response::IntoResponse {
    let payload: serde_json::Value = match serde_json::from_str(&body) {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, "Invalid WhatsApp webhook JSON");
            return (axum::http::StatusCode::BAD_REQUEST, "Invalid JSON");
        }
    };

    if payload.get("object").and_then(|v| v.as_str()) != Some("whatsapp_business_account") {
        return (axum::http::StatusCode::OK, "");
    }

    let empty: Vec<serde_json::Value> = vec![];
    let entries = payload.get("entry").and_then(|e| e.as_array()).map(|a| a.as_slice()).unwrap_or(&empty);
    for entry in entries {
        for msg in parse_whatsapp_webhook(entry) {
            let _ = state.tx.send(msg).await;
            let mut st = state.status.write().await;
            st.messages_received += 1;
            st.last_message_at = Some(Utc::now());
        }
    }

    (axum::http::StatusCode::OK, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_whatsapp_adapter_creation() {
        let adapter = WhatsAppAdapter::new(
            "123456".to_string(),
            "token".to_string(),
            "verify-123".to_string(),
            8080,
        );
        assert_eq!(adapter.name(), "whatsapp");
        assert_eq!(adapter.channel_type(), ChannelType::WhatsApp);
    }
}
