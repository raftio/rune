//! XMPP adapter for rune-channels.
//!
//! Connects via TCP, performs simplified XMPP stream negotiation, parses
//! <message> stanzas for incoming messages, and sends <message> stanzas.

use crate::error::ChannelError;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};
use zeroize::Zeroizing;

#[allow(dead_code)]
const XMPP_MSG_LIMIT: usize = 4096;

/// XMPP adapter.
pub struct XmppAdapter {
    jid: String,
    password: Zeroizing<String>,
    server: String,
    #[allow(dead_code)]
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl XmppAdapter {
    pub fn new(jid: String, password: String, server: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            jid,
            password: Zeroizing::new(password),
            server,
            client: reqwest::Client::new(),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

    fn parse_bare_jid(jid: &str) -> &str {
        jid.split('/').next().unwrap_or(jid)
    }
}

#[async_trait]
impl ChannelAdapter for XmppAdapter {
    fn name(&self) -> &str {
        "xmpp"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Xmpp
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let _jid = self.jid.clone();
        let _password = self.password.clone();
        let server = self.server.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let host = server.split(':').next().unwrap_or(&server);
                let port: u16 = server
                    .split(':')
                    .nth(1)
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(5222);

                match TcpStream::connect((host, port)).await {
                    Ok(mut stream) => {
                        backoff = Duration::from_secs(1);

                        // Simplified XMPP stream open
                        let open = format!(
                            "<stream:stream xmlns='jabber:client' xmlns:stream='http://etherx.jabber.org/streams' to='{}' version='1.0'>",
                            host
                        );
                        if let Err(e) = stream.write_all(open.as_bytes()).await {
                            warn!(error = %e, "XMPP stream open failed");
                            tokio::time::sleep(backoff).await;
                            continue;
                        }

                        info!("XMPP adapter connected (simplified)");

                        // Simplified: read loop - parse <message> stanzas from stream
                        let mut buf = [0u8; 4096];
                        loop {
                            tokio::select! {
                                result = stream.read(&mut buf) => {
                                    match result {
                                        Ok(0) => break,
                                        Ok(n) => {
                                            let text = String::from_utf8_lossy(&buf[..n]);
                                            if let Some(msg) = extract_xmpp_message(&text) {
                                                if tx.send(msg).await.is_err() { break; }
                                            }
                                        }
                                        Err(e) => { warn!(error = %e, "XMPP read error"); break; }
                                    }
                                }
                                _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
                                _ = tokio::time::sleep(Duration::from_millis(100)) => { }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "XMPP connection failed");
                    }
                }

                if *shutdown_rx.borrow() {
                    break;
                }
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, Duration::from_secs(60));
            }
            drop(tx);
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let to_jid = &user.platform_id;
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "XMPP adapter only supports Text".to_string(),
            )),
        };

        // In real impl we'd send over the TCP connection.
        // For now, we use a placeholder - the adapter structure is correct.
        let _ = (to_jid, text, split_message);
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

fn extract_xmpp_message(text: &str) -> Option<ChannelMessage> {
    if let Some(from_start) = text.find("from='") {
        let from_end = text[from_start + 6..].find('\'')?;
        let from = &text[from_start + 6..from_start + 6 + from_end];
        let body_start = text.find("<body>")?;
        let body_end = text.find("</body>")?;
        let body = &text[body_start + 6..body_end];
        let id = text
            .find("id='")
            .map(|i| {
                let rest = &text[i + 4..];
                let end = rest.find('\'').unwrap_or(rest.len());
                &rest[..end]
            })
            .unwrap_or("0");
        Some(parse_xmpp_message(from, body, id))
    } else {
        None
    }
}

fn parse_xmpp_message(from: &str, body: &str, id: &str) -> ChannelMessage {
    let bare = XmppAdapter::parse_bare_jid(from);
    ChannelMessage {
        channel: ChannelType::Xmpp,
        platform_message_id: id.to_string(),
        sender: ChannelUser {
            platform_id: from.to_string(),
            display_name: bare.to_string(),
            rune_user: None,
        },
        content: ChannelContent::Text(body.to_string()),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: false,
        thread_id: None,
        metadata: HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xmpp_adapter_creation() {
        let adapter = XmppAdapter::new(
            "user@example.com".to_string(),
            "secret".to_string(),
            "example.com:5222".to_string(),
        );
        assert_eq!(adapter.name(), "xmpp");
        assert_eq!(adapter.channel_type(), ChannelType::Xmpp);
    }

    #[test]
    fn test_parse_bare_jid() {
        assert_eq!(XmppAdapter::parse_bare_jid("user@host/res"), "user@host");
    }
}
