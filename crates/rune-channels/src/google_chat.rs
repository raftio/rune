//! Google Chat adapter.
//!
//! Receives webhook events via HTTP server and sends via
//! Google Chat API spaces.messages.create.

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

const GOOGLE_CHAT_API: &str = "https://chat.googleapis.com/v1";
const MAX_MSG_LEN: usize = 32768;

/// Reply context for Google Chat.
#[derive(Debug, Clone)]
struct GoogleChatReplyContext {
    space_name: String,
    thread_name: Option<String>,
}

/// Google Chat adapter.
pub struct GoogleChatAdapter {
    #[allow(dead_code)]
    project_id: String,
    credentials_json: Zeroizing<String>,
    webhook_port: u16,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    reply_context: Arc<RwLock<HashMap<String, GoogleChatReplyContext>>>,
    status: Arc<RwLock<ChannelStatus>>,
}

impl GoogleChatAdapter {
    /// Create a new Google Chat adapter.
    pub fn new(
        project_id: String,
        credentials_json: String,
        webhook_port: u16,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("reqwest client");

        Self {
            project_id,
            credentials_json: Zeroizing::new(credentials_json),
            webhook_port,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            reply_context: Arc::new(RwLock::new(HashMap::new())),
            status: Arc::new(RwLock::new(ChannelStatus::default())),
        }
    }

    async fn get_access_token(&self) -> Result<String, ChannelError> {
        let creds: serde_json::Value = serde_json::from_str(self.credentials_json.as_str())
            .map_err(|e| ChannelError::Config(e.to_string()))?;
        let creds = creds.as_object()
            .ok_or_else(|| ChannelError::Config("Invalid credentials JSON".into()))?;

        let client_email = creds
            .get("client_email")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::Config("Missing client_email".into()))?;
        let private_key = creds
            .get("private_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::Config("Missing private_key".into()))?;

        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = serde_json::json!({
            "iss": client_email,
            "sub": client_email,
            "aud": "https://oauth2.googleapis.com/token",
            "iat": now,
            "exp": now + 3600,
            "scope": "https://www.googleapis.com/auth/chat.messages"
        });
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(private_key.as_bytes())
            .map_err(|e| ChannelError::Auth(e.to_string()))?;
        let token = jsonwebtoken::encode(&header, &claims, &key)
            .map_err(|e| ChannelError::Auth(e.to_string()))?;

        let resp = self
            .client
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", token.as_str()),
            ])
            .send()
            .await
            .map_err(|e| ChannelError::Auth(e.to_string()))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ChannelError::Auth(e.to_string()))?;
        let access_token = json
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::Auth("No access_token".into()))?
            .to_string();
        Ok(access_token)
    }

    async fn api_create_message(
        &self,
        space_name: &str,
        text: &str,
        thread_name: Option<&str>,
    ) -> Result<(), ChannelError> {
        let token = self.get_access_token().await?;
        let chunks = split_message(text, MAX_MSG_LEN);

        for chunk in chunks {
            let mut body = serde_json::json!({
                "text": chunk
            });
            if let Some(thread) = thread_name {
                body["thread"] = serde_json::json!({ "name": thread });
            }

            let url = format!("{}/{}:messages", GOOGLE_CHAT_API, space_name.trim_end_matches('/'));

            let resp = self
                .client
                .post(&url)
                .bearer_auth(&token)
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

/// Parse a Google Chat webhook event into a ChannelMessage.
fn parse_google_chat_event(
    event: &serde_json::Value,
) -> Option<(ChannelMessage, GoogleChatReplyContext)> {
    let message = event.get("message")?.as_object()?;
    let sender = event.get("user")?.as_object()?;
    let space = event.get("space")?.as_object()?;

    let sender_name = sender
        .get("displayName")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let sender_id = sender.get("name")?.as_str()?.to_string();
    let space_name = space.get("name")?.as_str()?.to_string();
    let thread_name = message.get("thread").and_then(|t| t.get("name")).and_then(|n| n.as_str()).map(String::from);

    let text = message.get("text")?.as_str()?.to_string();
    let msg_name = message.get("name")?.as_str()?.to_string();
    let create_time = message
        .get("createTime")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let content = if text.starts_with('/') || text.starts_with('!') {
        let parts: Vec<&str> = text.splitn(2, ' ').collect();
        let cmd = parts[0].trim_start_matches('/').trim_start_matches('!');
        let name = cmd.to_string();
        let args: Vec<String> = if parts.len() > 1 {
            parts[1].split_whitespace().map(String::from).collect()
        } else {
            vec![]
        };
        ChannelContent::Command { name, args }
    } else {
        ChannelContent::Text(text.clone())
    };

    let mut metadata = HashMap::new();
    metadata.insert("space_name".to_string(), serde_json::json!(space_name.clone()));
    metadata.insert("thread_name".to_string(), serde_json::json!(thread_name.clone()));

    let thread_name_clone = thread_name.clone();

    let reply_ctx = GoogleChatReplyContext {
        space_name: space_name.clone(),
        thread_name,
    };

    Some((
        ChannelMessage {
            channel: ChannelType::GoogleChat,
            platform_message_id: msg_name,
            sender: ChannelUser {
                platform_id: format!("{}|{}", space_name, sender_id),
                display_name: sender_name.to_string(),
                rune_user: None,
            },
            content,
            target_agent: None,
            timestamp: create_time,
            is_group: true,
            thread_id: thread_name_clone,
            metadata,
        },
        reply_ctx,
    ))
}

#[async_trait]
impl ChannelAdapter for GoogleChatAdapter {
    fn name(&self) -> &str {
        "google_chat"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::GoogleChat
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let reply_context = self.reply_context.clone();
        let status = self.status.clone();
        let webhook_port = self.webhook_port;
        let mut shutdown_rx = self.shutdown_rx.clone();

        let app = axum::Router::new()
            .route("/webhook", axum::routing::post(google_chat_webhook))
            .with_state(Arc::new(GoogleChatWebhookState {
                tx,
                reply_context,
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
        info!(port = webhook_port, "Google Chat adapter started");

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let ctx = {
            let guard = self.reply_context.read().await;
            guard
                .get(&user.platform_id)
                .cloned()
                .ok_or_else(|| ChannelError::Send("No reply context for user".into()))?
        };

        match content {
            ChannelContent::Text(text) => {
                self.api_create_message(
                    &ctx.space_name,
                    &text,
                    ctx.thread_name.as_deref(),
                )
                .await?;
                let mut st = self.status.write().await;
                st.messages_sent += 1;
                Ok(())
            }
            _ => Err(ChannelError::UnsupportedContent(
                "Google Chat adapter only supports Text content".into(),
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

struct GoogleChatWebhookState {
    tx: mpsc::Sender<ChannelMessage>,
    reply_context: Arc<RwLock<HashMap<String, GoogleChatReplyContext>>>,
    status: Arc<RwLock<ChannelStatus>>,
}

async fn google_chat_webhook(
    axum::extract::State(state): axum::extract::State<Arc<GoogleChatWebhookState>>,
    body: String,
) -> impl axum::response::IntoResponse {
    let event: serde_json::Value = match serde_json::from_str(&body) {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "Invalid Google Chat webhook JSON");
            return (axum::http::StatusCode::BAD_REQUEST, "Invalid JSON");
        }
    };

    if event.get("type") != Some(&serde_json::json!("MESSAGE")) {
        return (axum::http::StatusCode::OK, "");
    }

    if let Some((msg, reply_ctx)) = parse_google_chat_event(&event) {
        {
            let mut guard = state.reply_context.write().await;
            guard.insert(msg.sender.platform_id.clone(), reply_ctx);
        }
        let _ = state.tx.send(msg).await;
        let mut st = state.status.write().await;
        st.messages_received += 1;
        st.last_message_at = Some(Utc::now());
    }

    (axum::http::StatusCode::OK, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_google_chat_adapter_creation() {
        let adapter = GoogleChatAdapter::new(
            "my-project".to_string(),
            r#"{"client_email":"x@y.iam.gserviceaccount.com","private_key":"-----BEGIN PRIVATE KEY-----\nxxx\n-----END PRIVATE KEY-----\n"}"#.to_string(),
            9090,
        );
        assert_eq!(adapter.name(), "google_chat");
        assert_eq!(adapter.channel_type(), ChannelType::GoogleChat);
    }
}
