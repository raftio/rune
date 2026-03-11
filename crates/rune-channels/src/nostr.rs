//! Nostr adapter for rune-channels.
//!
//! Connects to relay WebSocket, subscribes to kind 4 (DM) events.
//! Sends by publishing kind 4 events.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::{Stream, SinkExt, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tracing::{info, warn};
use zeroize::Zeroizing;

const KIND_DM: i64 = 4;
const MAX_MSG_LEN: usize = 4000;

/// Nostr adapter.
pub struct NostrAdapter {
    private_key: Zeroizing<String>,
    relays: Vec<String>,
    #[allow(dead_code)]
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    status: Arc<RwLock<ChannelStatus>>,
}

impl NostrAdapter {
    /// Create a new Nostr adapter.
    pub fn new(private_key: String, relays: Vec<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            private_key: Zeroizing::new(private_key),
            relays: if relays.is_empty() {
                vec!["wss://relay.damus.io".to_string()]
            } else {
                relays
            },
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            status: Arc::new(RwLock::new(ChannelStatus::default())),
        }
    }
}

#[async_trait]
impl ChannelAdapter for NostrAdapter {
    fn name(&self) -> &str {
        "nostr"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Nostr
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let relays = self.relays.clone();
        let private_key = self.private_key.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let status = self.status.clone();

        tokio::spawn(async move {
            let relay = relays.first().cloned().unwrap_or_else(|| "wss://relay.damus.io".to_string());

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                match connect_async(&relay).await {
                    Ok((ws_stream, _)) => {
                        status.write().await.connected = true;
                        status.write().await.started_at = Some(Utc::now());
                        info!(relay = %relay, "Nostr WebSocket connected");

                        let (mut write, mut read) = ws_stream.split();

                        let sub_id = format!("rune-{}", uuid::Uuid::new_v4());
                        let sub_msg = serde_json::json!([
                            "REQ",
                            sub_id,
                            { "kinds": [4i64] }
                        ]);
                        if write
                            .send(WsMessage::Text(sub_msg.to_string()))
                            .await
                            .is_err()
                        {
                            break;
                        }

                        let tx_clone = tx.clone();
                        loop {
                            tokio::select! {
                                Some(msg) = read.next() => {
                                    match msg {
                                        Ok(WsMessage::Text(text)) => {
                                            if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&text) {
                                                if arr.len() >= 3 && arr[0].as_str() == Some("EVENT") {
                                                    if let Some(ev) = arr.get(2) {
                                                        if let Some(parsed) = parse_nostr_event(ev, private_key.as_str()) {
                                                            if tx_clone.send(parsed).await.is_err() {
                                                                break;
                                                            }
                                                            status.write().await.messages_received += 1;
                                                            status.write().await.last_message_at = Some(Utc::now());
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        Ok(WsMessage::Close(_)) => break,
                                        Err(e) => {
                                            warn!(error = %e, "Nostr WebSocket error");
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                                _ = shutdown_rx.changed() => {
                                    if *shutdown_rx.borrow() {
                                        let _ = write.close().await;
                                        break;
                                    }
                                }
                            }
                        }

                        status.write().await.connected = false;
                    }
                    Err(e) => {
                        warn!(error = %e, relay = %relay, "Nostr WebSocket connection failed");
                        status.write().await.last_error = Some(e.to_string());
                        status.write().await.connected = false;
                    }
                }

                if *shutdown_rx.borrow() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => {
                return Err(ChannelError::UnsupportedContent(
                    "Nostr adapter only supports Text".to_string(),
                ))
            }
        };

        let pubkey = &user.platform_id;
        let chunks = split_message(&text, MAX_MSG_LEN);

        for chunk in chunks {
            let event = build_nostr_dm_event(
                self.private_key.as_str(),
                pubkey,
                &chunk,
            )?;

            for relay in &self.relays {
                let ws_url = relay
                    .replace("https://", "wss://")
                    .replace("http://", "ws://");
                if !ws_url.starts_with("wss://") && !ws_url.starts_with("ws://") {
                    continue;
                }

                if let Ok((mut ws, _)) = connect_async(&ws_url).await {
                    let msg = serde_json::json!(["EVENT", event]);
                    let _ = ws.send(WsMessage::Text(msg.to_string())).await;
                }
            }
        }

        self.status.write().await.messages_sent += 1;
        Ok(())
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        self.status.blocking_read().clone()
    }
}

fn parse_nostr_event(ev: &serde_json::Value, _our_sec_key: &str) -> Option<ChannelMessage> {
    let kind = ev.get("kind")?.as_i64()?;
    if kind != KIND_DM {
        return None;
    }

    let id = ev.get("id")?.as_str()?;
    let pubkey = ev.get("pubkey")?.as_str()?;
    let content = ev.get("content")?.as_str()?;
    let created_at = ev
        .get("created_at")
        .and_then(|v| v.as_i64())
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let mut metadata = HashMap::new();
    metadata.insert("pubkey".to_string(), serde_json::json!(pubkey));

    Some(ChannelMessage {
        channel: ChannelType::Nostr,
        platform_message_id: id.to_string(),
        sender: ChannelUser {
            platform_id: pubkey.to_string(),
            display_name: format!("{}…", &pubkey[..pubkey.len().min(8)]),
            rune_user: Some(pubkey.to_string()),
        },
        content: ChannelContent::Text(content.to_string()),
        target_agent: None,
        timestamp: created_at,
        is_group: false,
        thread_id: None,
        metadata,
    })
}

fn build_nostr_dm_event(
    sec_key: &str,
    pubkey: &str,
    content: &str,
) -> Result<serde_json::Value, ChannelError> {
    use secp256k1::{Message, Secp256k1, SecretKey};
    use sha2::{Digest, Sha256};

    let created_at = Utc::now().timestamp();
    let pubkey_hex = pubkey_from_secret(sec_key)
        .ok_or_else(|| ChannelError::Auth("invalid nostr private key".to_string()))?;

    let tags = serde_json::json!([["p", pubkey]]);
    let payload_array = serde_json::json!([0, pubkey_hex, created_at, KIND_DM, tags, content]);
    let payload_str = serde_json::to_string(&payload_array)
        .map_err(|e| ChannelError::Other(e.to_string()))?;

    let hash_bytes = Sha256::digest(payload_str.as_bytes());
    let hash_hex = hex::encode(hash_bytes);

    let sec_bytes = hex::decode(sec_key)
        .map_err(|_| ChannelError::Auth("invalid hex private key".to_string()))?;
    let sec = SecretKey::from_slice(&sec_bytes)
        .map_err(|_| ChannelError::Auth("invalid nostr private key".to_string()))?;
    let msg = Message::from_digest_slice(&hash_bytes)
        .map_err(|_| ChannelError::Other("message".into()))?;
    let sig = Secp256k1::new().sign_ecdsa(&msg, &sec);
    let sig_hex = hex::encode(sig.serialize_compact());

    Ok(serde_json::json!({
        "id": hash_hex,
        "pubkey": pubkey_hex,
        "created_at": created_at,
        "kind": KIND_DM,
        "tags": [["p", pubkey]],
        "content": content,
        "sig": sig_hex
    }))
}

fn pubkey_from_secret(sec_hex: &str) -> Option<String> {
    use secp256k1::PublicKey;
    let sec_bytes = hex::decode(sec_hex).ok()?;
    let sec = secp256k1::SecretKey::from_slice(&sec_bytes).ok()?;
    let secp = secp256k1::Secp256k1::new();
    let pk = PublicKey::from_secret_key(&secp, &sec);
    Some(hex::encode(pk.serialize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nostr_adapter_creation() {
        let adapter = NostrAdapter::new(
            "0000000000000000000000000000000000000000000000000000000000000001".to_string(),
            vec!["wss://relay.damus.io".to_string()],
        );
        assert_eq!(adapter.name(), "nostr");
        assert_eq!(adapter.channel_type(), ChannelType::Nostr);
    }
}
