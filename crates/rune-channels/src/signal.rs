//! Signal messenger adapter via signal-cli REST API.
//!
//! Polls GET /v1/receive/{number} for incoming messages and sends via
//! POST /v2/send with recipients.

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
use std::time::Duration;
use tokio::sync::{mpsc, watch, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};

const MAX_MSG_LEN: usize = 2000;
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Signal messenger adapter via signal-cli REST API.
pub struct SignalAdapter {
    api_url: String,
    phone_number: String,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    status: Arc<RwLock<ChannelStatus>>,
}

impl SignalAdapter {
    /// Create a new Signal adapter.
    pub fn new(api_url: String, phone_number: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");

        Self {
            api_url: api_url.trim_end_matches('/').to_string(),
            phone_number,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            status: Arc::new(RwLock::new(ChannelStatus::default())),
        }
    }

    #[allow(dead_code)]
    async fn receive_messages(&self) -> Result<Vec<serde_json::Value>, ChannelError> {
        let url = format!(
            "{}/v1/receive/{}",
            self.api_url,
            self.phone_number.replace('+', "")
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| ChannelError::Connection(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ChannelError::Connection(format!("{}: {}", status, text)));
        }

        let body: Vec<serde_json::Value> = resp
            .json()
            .await
            .map_err(|e| ChannelError::InvalidPayload(e.to_string()))?;
        Ok(body)
    }

    async fn api_send(&self, recipients: &[String], message: &str) -> Result<(), ChannelError> {
        let chunks = split_message(message, MAX_MSG_LEN);
        let url = format!("{}/v2/send", self.api_url);

        for chunk in chunks {
            let body = serde_json::json!({
                "recipients": recipients,
                "message": chunk
            });

            let resp = self
                .client
                .post(&url)
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

/// Parse a signal-cli receive envelope into ChannelMessage(s).
fn parse_signal_envelope(envelope: &serde_json::Value) -> Vec<ChannelMessage> {
    let mut messages = Vec::new();
    let Some(envelope_obj) = envelope.as_object() else {
        return messages;
    };
    let Some(source) = envelope_obj.get("sourceNumber").and_then(|n| n.as_str()).map(String::from) else {
        return messages;
    };
    let source_name = envelope_obj
        .get("sourceName")
        .and_then(|v| v.as_str())
        .unwrap_or(&source)
        .to_string();
    let timestamp = envelope_obj
        .get("timestamp")
        .and_then(|v| v.as_i64())
        .and_then(|ts| DateTime::from_timestamp(ts / 1000, ((ts % 1000) as u32) * 1_000_000))
        .unwrap_or_else(Utc::now);

    let data_message = envelope_obj.get("dataMessage");
    if let Some(data) = data_message {
        let Some(body) = data.get("message").and_then(|m| m.as_str()).map(String::from) else {
            return messages;
        };
        let id = envelope_obj
            .get("serverReceivedTimestamp")
            .and_then(|v| v.as_i64())
            .map(|t| t.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let content = if body.starts_with('/') {
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
        };

        let mut metadata = HashMap::new();
        metadata.insert("signal_source".to_string(), serde_json::json!(source));

        messages.push(ChannelMessage {
            channel: ChannelType::Signal,
            platform_message_id: id,
            sender: ChannelUser {
                platform_id: source,
                display_name: source_name,
                rune_user: None,
            },
            content,
            target_agent: None,
            timestamp,
            is_group: false,
            thread_id: None,
            metadata,
        });
    }

    messages
}

#[async_trait]
impl ChannelAdapter for SignalAdapter {
    fn name(&self) -> &str {
        "signal"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Signal
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let client = self.client.clone();
        let api_url = self.api_url.clone();
        let phone_number = self.phone_number.clone();
        let status = self.status.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            const MAX_BACKOFF: Duration = Duration::from_secs(60);

            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() { break; }
                    }
                    _ = tokio::time::sleep(Duration::ZERO) => {}
                }
                if *shutdown_rx.borrow() {
                    break;
                }

                let url = format!(
                    "{}/v1/receive/{}",
                    api_url,
                    phone_number.replace('+', "")
                );
                match client.get(&url).send().await {
                    Ok(resp) => {
                        backoff = Duration::from_secs(1);
                        if !resp.status().is_success() {
                            continue;
                        }
                        let envelopes: Vec<serde_json::Value> = match resp.json().await {
                            Ok(e) => e,
                            Err(_) => continue,
                        };
                        for envelope in envelopes {
                            for msg in parse_signal_envelope(&envelope) {
                                let _ = tx.send(msg).await;
                                let mut st = status.write().await;
                                st.messages_received += 1;
                                st.last_message_at = Some(Utc::now());
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Signal receive poll failed");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                    }
                }

                tokio::time::sleep(POLL_INTERVAL).await;
            }

            let mut st = status.write().await;
            st.connected = false;
        });

        {
            let mut st = self.status.write().await;
            st.connected = true;
            st.started_at = Some(Utc::now());
        }
        info!("Signal adapter started");

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        match content {
            ChannelContent::Text(text) => {
                self.api_send(&[user.platform_id.clone()], &text)
                    .await?;
                let mut st = self.status.write().await;
                st.messages_sent += 1;
                Ok(())
            }
            _ => Err(ChannelError::UnsupportedContent(
                "Signal adapter only supports Text content".into(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_adapter_creation() {
        let adapter = SignalAdapter::new(
            "http://localhost:8080".to_string(),
            "+1234567890".to_string(),
        );
        assert_eq!(adapter.name(), "signal");
        assert_eq!(adapter.channel_type(), ChannelType::Signal);
    }
}
