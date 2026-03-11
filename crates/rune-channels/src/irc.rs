//! IRC protocol adapter.
//!
//! Connects via TCP (optionally TLS), sends NICK/USER/JOIN,
//! parses PRIVMSG lines, and sends PRIVMSG for replies.

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
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};
use zeroize::Zeroizing;

const IRC_MSG_LIMIT: usize = 512;
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Outbound message to send over IRC.
#[derive(Clone)]
struct IrcOutbound {
    target: String,
    text: String,
}

/// IRC protocol adapter.
pub struct IrcAdapter {
    server: String,
    port: u16,
    nick: String,
    channels: Vec<String>,
    password: Option<Zeroizing<String>>,
    use_tls: bool,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    outbound_tx: Arc<RwLock<Option<mpsc::Sender<IrcOutbound>>>>,
    /// Maps platform_id -> target (channel or nick) for replying.
    reply_context: Arc<RwLock<HashMap<String, String>>>,
    status: Arc<RwLock<ChannelStatus>>,
}

impl IrcAdapter {
    /// Create a new IRC adapter.
    pub fn new(
        server: String,
        port: u16,
        nick: String,
        channels: Vec<String>,
        password: Option<String>,
        use_tls: bool,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            server,
            port,
            nick,
            channels,
            password: password.map(Zeroizing::new),
            use_tls,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            outbound_tx: Arc::new(RwLock::new(None)),
            reply_context: Arc::new(RwLock::new(HashMap::new())),
            status: Arc::new(RwLock::new(ChannelStatus::default())),
        }
    }
}

#[async_trait]
impl ChannelAdapter for IrcAdapter {
    fn name(&self) -> &str {
        "irc"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Irc
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let server = self.server.clone();
        let server_log = server.clone();
        let port = self.port;
        let nick = self.nick.clone();
        let channels = self.channels.clone();
        let password = self.password.clone();
        let use_tls = self.use_tls;
        let reply_context = self.reply_context.clone();
        let status = self.status.clone();
        let outbound_tx = self.outbound_tx.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let (out_tx, mut out_rx) = mpsc::channel::<IrcOutbound>(32);
                {
                    let mut guard = outbound_tx.write().await;
                    *guard = Some(out_tx);
                }

                match run_irc_connection(
                    &server,
                    port,
                    &nick,
                    &channels,
                    password.as_ref().map(|s| s.as_str()),
                    use_tls,
                    &reply_context,
                    &status,
                    &mut shutdown_rx,
                    &mut out_rx,
                    &tx,
                )
                .await
                {
                    Ok(()) => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                        backoff = Duration::from_secs(1);
                        warn!("IRC disconnected, reconnecting...");
                    }
                    Err(e) => {
                        warn!(error = %e, "IRC connection error");
                    }
                }

                if *shutdown_rx.borrow() {
                    break;
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
            outbound_tx.write().await.take();
            drop(tx);
        });

        {
            let mut st = self.status.write().await;
            st.connected = true;
            st.started_at = Some(Utc::now());
        }
        info!(server = %server_log, "IRC adapter started");

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let target = {
            let guard = self.reply_context.read().await;
            guard.get(&user.platform_id).cloned()
        };
        let target = target
            .or_else(|| user.platform_id.split('|').next().map(String::from))
            .ok_or_else(|| ChannelError::Send("No reply target for user".into()))?;

        match content {
            ChannelContent::Text(text) => {
                let out_tx = {
                    let guard = self.outbound_tx.read().await;
                    guard.clone()
                };
                let out_tx = out_tx.ok_or_else(|| ChannelError::Send("IRC not connected".into()))?;
                let chunks = split_message(&text, IRC_MSG_LIMIT.saturating_sub(20));
                for chunk in chunks {
                    let msg = chunk.replace('\r', "").replace('\n', " ");
                    out_tx.send(IrcOutbound { target: target.clone(), text: msg }).await
                        .map_err(|_| ChannelError::Send("IRC connection closed".into()))?;
                }
                let mut st = self.status.write().await;
                st.messages_sent += 1;
                Ok(())
            }
            _ => Err(ChannelError::UnsupportedContent(
                "IRC adapter only supports Text content".into(),
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

async fn run_irc_connection(
    server: &str,
    port: u16,
    nick: &str,
    channels: &[String],
    password: Option<&str>,
    _use_tls: bool,
    reply_context: &Arc<RwLock<HashMap<String, String>>>,
    status: &Arc<RwLock<ChannelStatus>>,
    shutdown_rx: &mut watch::Receiver<bool>,
    out_rx: &mut mpsc::Receiver<IrcOutbound>,
    tx: &mpsc::Sender<ChannelMessage>,
) -> Result<(), ChannelError> {
    let addr = format!("{}:{}", server, port);
    let stream = TcpStream::connect(&addr)
        .await
        .map_err(|e| ChannelError::Connection(e.to_string()))?;

    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    if let Some(pass) = password {
        let _ = writer.write_all(format!("PASS {}\r\n", pass).as_bytes()).await;
    }
    let _ = writer.write_all(format!("NICK {}\r\n", nick).as_bytes()).await;
    let _ = writer.write_all(format!("USER {} 0 * :{}\r\n", nick, nick).as_bytes()).await;
    let _ = writer.flush().await;

    for ch in channels {
        let _ = writer.write_all(format!("JOIN {}\r\n", ch).as_bytes()).await;
    }
    let _ = writer.flush().await;

    let mut line = String::new();
    loop {
        tokio::select! {
            result = reader.read_line(&mut line) => {
                if result.unwrap_or(0) == 0 {
                    return Ok(());
                }
                let trimmed = line.trim_end_matches(|c| c == '\r' || c == '\n');
                if trimmed.starts_with("PING") {
                    let reply = trimmed.strip_prefix("PING").unwrap_or("").trim();
                    let _ = writer.write_all(format!("PONG {}\r\n", reply).as_bytes()).await;
                    let _ = writer.flush().await;
                } else if let Some(msg) = parse_irc_privmsg(trimmed, nick) {
                    let reply_target = msg.metadata.get("reply_target").and_then(|v| v.as_str()).unwrap_or("");
                    if !reply_target.is_empty() {
                        reply_context.write().await.insert(msg.sender.platform_id.clone(), reply_target.to_string());
                    }
                    let _ = tx.send(msg).await;
                    status.write().await.messages_received += 1;
                    status.write().await.last_message_at = Some(Utc::now());
                }
                line.clear();
            }
            out_msg = out_rx.recv() => {
                if let Some(IrcOutbound { target, text }) = out_msg {
                    let line = format!("PRIVMSG {} :{}\r\n", target, text);
                    let _ = writer.write_all(line.as_bytes()).await;
                    let _ = writer.flush().await;
                }
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    return Ok(());
                }
            }
        }
    }
}

/// Parse an IRC PRIVMSG line into a ChannelMessage.
fn parse_irc_privmsg(line: &str, our_nick: &str) -> Option<ChannelMessage> {
    let line = line.trim();
    if !line.starts_with(":") {
        return None;
    }
    let rest = line.trim_start_matches(':');
    let mut parts = rest.splitn(2, ' ');
    let source = parts.next()?;
    let rest = parts.next()?;
    let mut parts = rest.splitn(3, ' ');
    let command = parts.next()?;
    if command != "PRIVMSG" {
        return None;
    }
    let target = parts.next()?;
    let msg = parts.next().unwrap_or("");
    let msg = msg.strip_prefix(':').unwrap_or(msg);
    if msg.is_empty() {
        return None;
    }

    let (nick, _) = source.split_once('!').unwrap_or((source, ""));
    if nick == our_nick {
        return None;
    }

    let is_channel = target.starts_with('#') || target.starts_with('&');
    let reply_target = if is_channel { target } else { nick };
    let platform_id = if is_channel {
        format!("{}|{}", target, nick)
    } else {
        nick.to_string()
    };

    let content = if msg.starts_with('\x01') && msg.ends_with('\x01') {
        let ctcp = msg.trim_matches('\x01');
        let parts: Vec<&str> = ctcp.splitn(2, ' ').collect();
        let name = parts[0].to_lowercase();
        let args: Vec<String> = if parts.len() > 1 {
            parts[1].split_whitespace().map(String::from).collect()
        } else {
            vec![]
        };
        ChannelContent::Command { name, args }
    } else if msg.starts_with("!") || msg.starts_with(".") {
        let parts: Vec<&str> = msg.splitn(2, ' ').collect();
        let name = parts[0].trim_start_matches('!').trim_start_matches('.').to_string();
        let args: Vec<String> = if parts.len() > 1 {
            parts[1].split_whitespace().map(String::from).collect()
        } else {
            vec![]
        };
        ChannelContent::Command { name, args }
    } else {
        ChannelContent::Text(msg.to_string())
    };

    let mut metadata = HashMap::new();
    metadata.insert("reply_target".to_string(), serde_json::json!(reply_target));
    metadata.insert("source".to_string(), serde_json::json!(source));

    Some(ChannelMessage {
        channel: ChannelType::Irc,
        platform_message_id: format!("irc-{}-{}", target, Utc::now().timestamp()),
        sender: ChannelUser {
            platform_id: platform_id.clone(),
            display_name: nick.to_string(),
            rune_user: None,
        },
        content,
        target_agent: None,
        timestamp: Utc::now(),
        is_group: is_channel,
        thread_id: Some(reply_target.to_string()),
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_irc_adapter_creation() {
        let adapter = IrcAdapter::new(
            "irc.example.com".to_string(),
            6667,
            "rune_bot".to_string(),
            vec!["#test".to_string()],
            None,
            false,
        );
        assert_eq!(adapter.name(), "irc");
        assert_eq!(adapter.channel_type(), ChannelType::Irc);
    }

    #[test]
    fn test_parse_irc_privmsg() {
        let line = ":alice!user@host PRIVMSG #channel :Hello world";
        let msg = parse_irc_privmsg(line, "rune_bot");
        assert!(msg.is_some());
        let m = msg.unwrap();
        assert_eq!(m.channel, ChannelType::Irc);
        assert_eq!(m.sender.display_name, "alice");
        assert!(matches!(m.content, ChannelContent::Text(ref t) if t == "Hello world"));
    }
}
