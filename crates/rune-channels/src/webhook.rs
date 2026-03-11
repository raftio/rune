//! Generic HTTP webhook channel adapter for rune-channels.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use axum::response::IntoResponse;
use chrono::Utc;
use futures::Stream;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};
use zeroize::Zeroizing;

/// Maximum message length for splitting outbound messages.
pub const MAX_MESSAGE_LEN: usize = 65535;

type HmacSha256 = Hmac<Sha256>;

/// Result of parsing a webhook JSON body.
#[derive(Debug, Clone)]
pub struct WebhookPayload {
    pub sender_id: String,
    pub sender_name: String,
    pub message: String,
    pub thread_id: Option<String>,
    pub is_group: bool,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Constant-time comparison of two byte slices.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Compute HMAC-SHA256 signature of data, returning hex-encoded string.
pub fn compute_signature(secret: &str, data: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC-SHA256 accepts any key size");
    mac.update(data);
    hex::encode(mac.finalize().into_bytes())
}

/// Verify webhook signature using constant-time comparison.
/// Expects header value in form "sha256=hexdigest" or just "hexdigest".
pub fn verify_signature(secret: &str, body: &[u8], signature: &str) -> bool {
    let expected = compute_signature(secret, body);
    let sig = signature.strip_prefix("sha256=").unwrap_or(signature).trim();
    constant_time_eq(expected.as_bytes(), sig.as_bytes())
}

/// Parse a webhook JSON body into structured fields.
/// Supports common field names: sender_id/user_id/from, sender_name/display_name/name,
/// message/text/body/content, thread_id, is_group/group, metadata.
pub fn parse_webhook_body(body: &serde_json::Value) -> Result<WebhookPayload, ChannelError> {
    let obj = body
        .as_object()
        .ok_or_else(|| ChannelError::InvalidPayload("expected JSON object".to_string()))?;

    let sender_id = obj
        .get("sender_id")
        .or_else(|| obj.get("user_id"))
        .or_else(|| obj.get("from"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let sender_name = obj
        .get("sender_name")
        .or_else(|| obj.get("display_name"))
        .or_else(|| obj.get("name"))
        .or_else(|| obj.get("from_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let message = obj
        .get("message")
        .or_else(|| obj.get("text"))
        .or_else(|| obj.get("body"))
        .or_else(|| obj.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let thread_id = obj
        .get("thread_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    let is_group = obj
        .get("is_group")
        .or_else(|| obj.get("group"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let metadata = obj
        .get("metadata")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.clone())))
                .collect()
        })
        .unwrap_or_default();

    Ok(WebhookPayload {
        sender_id,
        sender_name,
        message,
        thread_id,
        is_group,
        metadata,
    })
}

/// Generic HTTP webhook channel adapter.
pub struct WebhookAdapter {
    secret: Zeroizing<String>,
    listen_port: u16,
    callback_url: Option<String>,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl WebhookAdapter {
    /// Create a new webhook adapter.
    pub fn new(
        secret: String,
        listen_port: u16,
        callback_url: Option<String>,
    ) -> Result<Self, ChannelError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ChannelError::Config(e.to_string()))?;

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Ok(Self {
            secret: Zeroizing::new(secret),
            listen_port,
            callback_url,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        })
    }

    /// Whether this adapter has an outbound callback URL configured.
    pub fn has_callback(&self) -> bool {
        self.callback_url.is_some()
    }
}

#[async_trait]
impl ChannelAdapter for WebhookAdapter {
    fn name(&self) -> &str {
        "webhook"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("webhook".to_string())
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let secret = self.secret.clone();
        let shutdown_rx = self.shutdown_rx.clone();
        let listen_port = self.listen_port;

        let app = axum::Router::new()
            .route("/webhook", axum::routing::post(webhook_handler))
            .route("/", axum::routing::post(webhook_handler))
            .with_state(WebhookState {
                secret: secret.clone(),
                tx: tx.clone(),
            });

        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", listen_port))
            .await
            .map_err(|e| ChannelError::Config(format!("bind to port {}: {}", listen_port, e)))?;

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
                error!(error = %e, "webhook server stopped with error");
            }
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), ChannelError> {
        let url = self
            .callback_url
            .as_ref()
            .ok_or_else(|| ChannelError::Config("no callback_url configured".to_string()))?;

        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent("webhook only supports Text".to_string())),
        };

        let chunks = split_message(&text, MAX_MESSAGE_LEN);
        for chunk in chunks {
            let payload = serde_json::json!({
                "sender_id": user.platform_id,
                "sender_name": user.display_name,
                "message": chunk,
                "metadata": {}
            });
            let body = serde_json::to_vec(&payload)?;
            let signature = compute_signature(&self.secret, &body);
            let sig_header = format!("sha256={}", signature);

            let resp = self
                .client
                .post(url)
                .header("X-Webhook-Signature", sig_header)
                .header("Content-Type", "application/json")
                .body(body)
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
            debug!(user = %user.platform_id, "webhook message sent");
        }
        Ok(())
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        let _ = self.shutdown_tx.send(true);
        info!("webhook adapter shutdown signal sent");
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        ChannelStatus {
            connected: true,
            ..ChannelStatus::default()
        }
    }
}

#[derive(Clone)]
struct WebhookState {
    secret: Zeroizing<String>,
    tx: mpsc::Sender<ChannelMessage>,
}

async fn webhook_handler(
    axum::extract::State(state): axum::extract::State<WebhookState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl axum::response::IntoResponse {
    let signature = headers
        .get("X-Webhook-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify_signature(&state.secret, &body, signature) {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "invalid signature" })),
        )
            .into_response();
    }

    let value: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({ "error": format!("invalid JSON: {}", e) })),
            )
                .into_response();
        }
    };

    let payload = match parse_webhook_body(&value) {
        Ok(p) => p,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let msg = ChannelMessage {
        channel: ChannelType::Custom("webhook".to_string()),
        platform_message_id: uuid::Uuid::new_v4().to_string(),
        sender: ChannelUser {
            platform_id: payload.sender_id.clone(),
            display_name: payload.sender_name.clone(),
            rune_user: None,
        },
        content: ChannelContent::Text(payload.message),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: payload.is_group,
        thread_id: payload.thread_id,
        metadata: payload.metadata,
    };

    if state.tx.send(msg).await.is_err() {
        warn!("webhook channel closed, dropping message");
    }

    (
        axum::http::StatusCode::OK,
        axum::Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_signature_computation() {
        let sig = compute_signature("mysecret", b"hello world");
        assert!(!sig.is_empty());
        assert_eq!(sig.len(), 64); // 32 bytes = 64 hex chars
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_webhook_signature_verification() {
        let secret = "test-secret";
        let body = b"{\"message\":\"hi\"}";
        let sig = compute_signature(secret, body);
        assert!(verify_signature(secret, body, &format!("sha256={}", sig)));
        assert!(verify_signature(secret, body, &sig));
        assert!(!verify_signature(secret, body, "sha256=deadbeef"));
        assert!(!verify_signature("wrong-secret", body, &format!("sha256={}", sig)));
        assert!(!verify_signature(secret, b"different", &format!("sha256={}", sig)));
    }

    #[test]
    fn test_webhook_parse_body_full() {
        let body = serde_json::json!({
            "sender_id": "user123",
            "sender_name": "Alice",
            "message": "Hello",
            "thread_id": "thread_abc",
            "is_group": true,
            "metadata": { "key": "value" }
        });
        let p = parse_webhook_body(&body).unwrap();
        assert_eq!(p.sender_id, "user123");
        assert_eq!(p.sender_name, "Alice");
        assert_eq!(p.message, "Hello");
        assert_eq!(p.thread_id, Some("thread_abc".to_string()));
        assert!(p.is_group);
        assert_eq!(p.metadata.get("key").and_then(|v| v.as_str()), Some("value"));
    }

    #[test]
    fn test_webhook_parse_body_minimal() {
        let body = serde_json::json!({
            "text": "hi",
            "user_id": "u1"
        });
        let p = parse_webhook_body(&body).unwrap();
        assert_eq!(p.sender_id, "u1");
        assert_eq!(p.sender_name, "");
        assert_eq!(p.message, "hi");
        assert_eq!(p.thread_id, None);
        assert!(!p.is_group);
        assert!(p.metadata.is_empty());
    }

    #[test]
    fn test_webhook_adapter_creation() {
        let adapter = WebhookAdapter::new(
            "secret".to_string(),
            9999,
            Some("https://example.com/cb".to_string()),
        )
        .unwrap();
        assert_eq!(adapter.name(), "webhook");
        assert!(adapter.has_callback());

        let no_cb = WebhookAdapter::new("secret".to_string(), 8888, None).unwrap();
        assert!(!no_cb.has_callback());
    }
}
