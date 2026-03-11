//! DingTalk adapter for rune-channels.
//!
//! Receives events via HTTP webhook callbacks and sends messages via the
//! DingTalk Robot Send API with HMAC-SHA256 signature.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use axum::{
    extract::State,
    routing::post,
    Json, Router,
};
use chrono::Utc;
use futures::Stream;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};
use zeroize::Zeroizing;

type HmacSha256 = Hmac<Sha256>;

const DINGTALK_API: &str = "https://oapi.dingtalk.com/robot/send";
const MAX_MSG_LEN: usize = 20000;

/// DingTalk Robot adapter.
pub struct DingTalkAdapter {
    #[allow(dead_code)]
    app_key: String,
    app_secret: Zeroizing<String>,
    webhook_port: u16,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

#[derive(Clone)]
struct DingTalkState {
    tx: mpsc::Sender<ChannelMessage>,
}

impl DingTalkAdapter {
    /// Create a new DingTalk adapter.
    pub fn new(
        app_key: String,
        app_secret: String,
        webhook_port: u16,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            app_key,
            app_secret: Zeroizing::new(app_secret),
            webhook_port,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

    fn compute_sign(&self, timestamp: i64) -> Result<String, ChannelError> {
        let string_to_sign = format!("{}\n{}", timestamp, self.app_secret.as_str());
        let mut mac = HmacSha256::new_from_slice(self.app_secret.as_bytes())
            .map_err(|e| ChannelError::Config(e.to_string()))?;
        mac.update(string_to_sign.as_bytes());
        let sign = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        Ok(urlencoding::encode(&sign).into_owned())
    }
}

#[async_trait]
impl ChannelAdapter for DingTalkAdapter {
    fn name(&self) -> &str {
        "dingtalk"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::DingTalk
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let shutdown_rx = self.shutdown_rx.clone();
        let webhook_port = self.webhook_port;

        let app = Router::new()
            .route("/dingtalk/webhook", post(dingtalk_webhook_handler))
            .route("/dingtalk", post(dingtalk_webhook_handler))
            .with_state(DingTalkState { tx: tx.clone() });

        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", webhook_port))
            .await
            .map_err(|e| ChannelError::Config(format!("bind to port {}: {}", webhook_port, e)))?;

        let mut shutdown_rx_clone = shutdown_rx.clone();
        let shutdown = async move {
            loop {
                if *shutdown_rx_clone.borrow() {
                    break;
                }
                if shutdown_rx_clone.changed().await.is_err() {
                    break;
                }
            }
        };

        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await
            {
                warn!(error = %e, "DingTalk webhook server stopped");
            }
        });

        info!(port = %webhook_port, "DingTalk adapter started");
        let stream = ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "DingTalk adapter only supports Text content".to_string(),
            )),
        };

        let timestamp = Utc::now().timestamp_millis();
        let sign = self.compute_sign(timestamp)?;

        let access_token = &user.platform_id;
        let url = format!(
            "{}?access_token={}&timestamp={}&sign={}",
            DINGTALK_API,
            access_token,
            timestamp,
            sign
        );

        let chunks = split_message(&text, MAX_MSG_LEN);
        for chunk in chunks {
            let body = serde_json::json!({
                "msgtype": "text",
                "text": { "content": chunk }
            });

            let resp = self
                .client
                .post(&url)
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_body = resp.text().await.unwrap_or_default();
                return Err(ChannelError::Send(format!(
                    "HTTP {}: {}",
                    status,
                    err_body.lines().next().unwrap_or(&err_body)
                )));
            }

            let json: serde_json::Value = resp.json().await?;
            if json.get("errcode").and_then(|v| v.as_i64()) != Some(0) {
                let errmsg = json
                    .get("errmsg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                return Err(ChannelError::Send(errmsg.to_string()));
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

async fn dingtalk_webhook_handler(
    State(state): State<DingTalkState>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let sender_id = body
        .get("senderId")
        .or_else(|| body.get("senderId"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let sender_name = body
        .get("senderNick")
        .or_else(|| body.get("senderNickname"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let text = body
        .get("text")
        .and_then(|v| v.as_object())
        .and_then(|o| o.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if text.is_empty() {
        return Json(serde_json::json!({ "ok": true }));
    }

    let msg = ChannelMessage {
        channel: ChannelType::DingTalk,
        platform_message_id: uuid::Uuid::new_v4().to_string(),
        sender: ChannelUser {
            platform_id: sender_id.clone(),
            display_name: sender_name,
            rune_user: None,
        },
        content: ChannelContent::Text(text),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: true,
        thread_id: None,
        metadata: HashMap::new(),
    };

    if state.tx.send(msg).await.is_err() {
        warn!("DingTalk channel closed, dropping message");
    }

    Json(serde_json::json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dingtalk_adapter_creation() {
        let adapter = DingTalkAdapter::new(
            "app_key".to_string(),
            "app_secret".to_string(),
            19000,
        );
        assert_eq!(adapter.name(), "dingtalk");
        assert_eq!(adapter.channel_type(), ChannelType::DingTalk);
    }
}
