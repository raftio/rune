//! Microsoft Teams adapter using Bot Framework REST API.
//!
//! Receives activities via HTTP webhook (POST /api/messages) and sends replies
//! using the Bot Framework REST API.

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

const MAX_MSG_LEN: usize = 28000;

/// Reply context for Teams: service URL and bot id from the incoming activity.
#[derive(Debug, Clone)]
struct TeamsReplyContext {
    service_url: String,
    conversation_id: String,
    bot_id: String,
}

/// Microsoft Teams adapter using Bot Framework REST API.
pub struct TeamsAdapter {
    app_id: String,
    app_password: Zeroizing<String>,
    service_url: String,
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    reply_context: Arc<RwLock<HashMap<String, TeamsReplyContext>>>,
    status: Arc<RwLock<ChannelStatus>>,
}

impl TeamsAdapter {
    /// Create a new Teams adapter.
    pub fn new(
        app_id: String,
        app_password: String,
        service_url: String,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("reqwest client");

        Self {
            app_id,
            app_password: Zeroizing::new(app_password),
            service_url,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            reply_context: Arc::new(RwLock::new(HashMap::new())),
            status: Arc::new(RwLock::new(ChannelStatus::default())),
        }
    }

    async fn get_bearer_token(&self) -> Result<String, ChannelError> {
        let url = "https://login.microsoftonline.com/botframework.com/oauth2/v2.0/token";
        let params = [
            ("grant_type", "client_credentials"),
            ("client_id", self.app_id.as_str()),
            ("client_secret", self.app_password.as_str()),
            ("scope", "https://api.botframework.com/.default"),
        ];
        let resp = self
            .client
            .post(url)
            .form(&params)
            .send()
            .await
            .map_err(|e| ChannelError::Auth(e.to_string()))?;
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ChannelError::Auth(e.to_string()))?;
        let token = json
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::Auth("No access_token in token response".into()))?
            .to_string();
        Ok(token)
    }

    async fn api_send_message(
        &self,
        service_url: &str,
        conversation_id: &str,
        bot_id: &str,
        text: &str,
    ) -> Result<(), ChannelError> {
        let token = self.get_bearer_token().await?;
        let chunks = split_message(text, MAX_MSG_LEN);

        for chunk in chunks {
            let body = serde_json::json!({
                "type": "message",
                "from": { "id": bot_id },
                "conversation": { "id": conversation_id },
                "text": chunk
            });

            let url = format!(
                "{}/v3/conversations/{}/activities",
                service_url.trim_end_matches('/'),
                conversation_id
            );

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

/// Parse a Bot Framework activity into a ChannelMessage.
/// Returns (ChannelMessage, TeamsReplyContext) for storing reply context.
fn parse_teams_activity(
    activity: &serde_json::Value,
) -> Option<(ChannelMessage, TeamsReplyContext)> {
    let msg_type = activity.get("type")?.as_str()?;
    if msg_type != "message" {
        return None;
    }

    let text = activity.get("text")?.as_str()?;
    let from = activity.get("from")?.as_object()?;
    let from_id = from.get("id")?.as_str()?.to_string();
    let from_name = from
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&from_id)
        .to_string();

    let id = activity
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .to_string();

    let conversation = activity.get("conversation")?.as_object()?;
    let conversation_id = conversation.get("id")?.as_str()?.to_string();
    let is_group = conversation
        .get("conversationType")
        .and_then(|v| v.as_str())
        != Some("personal");

    let timestamp = activity
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let service_url = activity
        .get("serviceUrl")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let recipient = activity.get("recipient")?.as_object()?;
    let bot_id = recipient.get("id")?.as_str()?.to_string();

    let mut metadata = HashMap::new();
    metadata.insert(
        "conversation_id".to_string(),
        serde_json::json!(conversation_id),
    );

    let content = if text.starts_with('/') {
        let parts: Vec<&str> = text.splitn(2, ' ').collect();
        let name = parts[0].strip_prefix('/').unwrap_or("").to_string();
        let args: Vec<String> = if parts.len() > 1 {
            parts[1].split_whitespace().map(String::from).collect()
        } else {
            vec![]
        };
        ChannelContent::Command { name, args }
    } else {
        ChannelContent::Text(text.to_string())
    };

    let reply_ctx = TeamsReplyContext {
        service_url,
        conversation_id: conversation_id.clone(),
        bot_id,
    };

    Some((
        ChannelMessage {
            channel: ChannelType::Teams,
            platform_message_id: id,
            sender: ChannelUser {
                platform_id: conversation_id,
                display_name: from_name,
                rune_user: None,
            },
            content,
            target_agent: None,
            timestamp,
            is_group,
            thread_id: activity
                .get("replyToId")
                .and_then(|v| v.as_str())
                .map(String::from),
            metadata,
        },
        reply_ctx,
    ))
}

#[async_trait]
impl ChannelAdapter for TeamsAdapter {
    fn name(&self) -> &str {
        "teams"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Teams
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let app_id = self.app_id.clone();
        let app_password = self.app_password.clone();
        let service_url = self.service_url.clone();
        let client = self.client.clone();
        let status = self.status.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        let router = axum::Router::new()
            .route("/api/messages", axum::routing::post(teams_webhook_handler));

        let listener = tokio::net::TcpListener::bind("0.0.0.0:3978")
            .await
            .map_err(|e| ChannelError::Config(e.to_string()))?;

        let reply_context = self.reply_context.clone();
        let app_state = TeamsWebhookState {
            tx,
            app_id,
            app_password,
            service_url,
            client,
            reply_context,
            status,
        };

        let app = router.with_state(Arc::new(app_state));

        tokio::spawn(async move {
            let server = axum::serve(listener, app);
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
        info!("Teams adapter started, listening on :3978/api/messages");

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let ctx = {
            let guard = self.reply_context.read().await;
            guard
                .get(&user.platform_id)
                .cloned()
                .ok_or_else(|| ChannelError::Send("No reply context for conversation".into()))?
        };

        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(
                    &ctx.service_url,
                    &ctx.conversation_id,
                    &ctx.bot_id,
                    &text,
                )
                .await?;
                let mut st = self.status.write().await;
                st.messages_sent += 1;
                Ok(())
            }
            _ => Err(ChannelError::UnsupportedContent(
                "Teams adapter only supports Text content".into(),
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

struct TeamsWebhookState {
    tx: mpsc::Sender<ChannelMessage>,
    #[allow(dead_code)]
    app_id: String,
    #[allow(dead_code)]
    app_password: Zeroizing<String>,
    #[allow(dead_code)]
    service_url: String,
    #[allow(dead_code)]
    client: reqwest::Client,
    reply_context: Arc<RwLock<HashMap<String, TeamsReplyContext>>>,
    status: Arc<RwLock<ChannelStatus>>,
}

async fn teams_webhook_handler(
    axum::extract::State(state): axum::extract::State<Arc<TeamsWebhookState>>,
    body: String,
) -> impl axum::response::IntoResponse {
    let activity: serde_json::Value = match serde_json::from_str(&body) {
        Ok(a) => a,
        Err(e) => {
            warn!(error = %e, "Invalid Teams activity JSON");
            return (axum::http::StatusCode::BAD_REQUEST, "Invalid JSON");
        }
    };

    if let Some((msg, reply_ctx)) = parse_teams_activity(&activity) {
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
    fn test_teams_adapter_creation() {
        let adapter = TeamsAdapter::new(
            "app-id".to_string(),
            "app-secret".to_string(),
            "https://smba.trafficmanager.net/amer/".to_string(),
        );
        assert_eq!(adapter.name(), "teams");
        assert_eq!(adapter.channel_type(), ChannelType::Teams);
    }

    #[test]
    fn test_parse_teams_activity() {
        let activity = serde_json::json!({
            "type": "message",
            "id": "msg123",
            "from": { "id": "user1", "name": "Alice" },
            "recipient": { "id": "bot1" },
            "conversation": { "id": "conv1", "conversationType": "personal" },
            "serviceUrl": "https://example.com",
            "text": "Hello",
            "timestamp": "2024-01-15T12:00:00Z"
        });
        let result = parse_teams_activity(&activity);
        assert!(result.is_some());
        let (m, ctx) = result.unwrap();
        assert_eq!(m.channel, ChannelType::Teams);
        assert_eq!(m.platform_message_id, "msg123");
        assert_eq!(m.sender.platform_id, "conv1");
        assert_eq!(ctx.conversation_id, "conv1");
        assert_eq!(ctx.bot_id, "bot1");
    }
}
