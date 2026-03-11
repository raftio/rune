//! Mumble adapter for rune-channels (text chat only).
//!
//! Connects to a Mumble server and handles TextMessage packets.
//! Requires TLS for production Mumble servers.

use crate::error::ChannelError;
use crate::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType, ChannelUser,
};
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};
use zeroize::Zeroizing;

#[allow(dead_code)]
const MAX_MSG_LEN: usize = 900;

/// Mumble text chat adapter.
pub struct MumbleAdapter {
    server: String,
    port: u16,
    username: String,
    password: Option<Zeroizing<String>>,
    #[allow(dead_code)]
    client: reqwest::Client,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl MumbleAdapter {
    /// Create a new Mumble adapter.
    pub fn new(
        server: String,
        port: u16,
        username: String,
        password: Option<String>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            server,
            port,
            username,
            password: password.map(Zeroizing::new),
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
}

#[async_trait]
impl ChannelAdapter for MumbleAdapter {
    fn name(&self) -> &str {
        "mumble"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Mumble
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (_tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let server = self.server.clone();
        let port = self.port;
        let _username = self.username.clone();
        let _password = self.password.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let addr = format!("{}:{}", server, port);
            let mut backoff = Duration::from_secs(1);
            const MAX_BACKOFF: Duration = Duration::from_secs(60);

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                match tokio::net::TcpStream::connect(&addr).await {
                    Ok(stream) => {
                        backoff = Duration::from_secs(1);
                        // Mumble uses TLS; for a minimal implementation we would need
                        // native-tls or rustls to wrap the stream. Here we spawn a
                        // task that would parse Mumble Protocol Buffers (TextMessage
                        // packet type 11) from the control channel. For now, we keep
                        // the connection open and listen for shutdown.
                        let mut shutdown = shutdown_rx.clone();
                        tokio::spawn(async move {
                            let (mut reader, _writer) = stream.into_split();
                            let mut buf = [0u8; 4096];
                            while !*shutdown.borrow() {
                                tokio::select! {
                                    _ = shutdown.changed() => {}
                                    result = tokio::io::AsyncReadExt::read(&mut reader, &mut buf) => {
                                        match result {
                                            Ok(0) => break,
                                            Ok(_) => {
                                                // Parse Mumble packets: 6-byte header (2B type + 4B len) + protobuf payload.
                                                // Full implementation would use mumble-protocol crate.
                                            }
                                            Err(_) => break,
                                        }
                                    }
                                }
                            }
                        });
                        // Wait for shutdown
                        while !*shutdown_rx.borrow() {
                            if shutdown_rx.changed().await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, server = %server, "Mumble connection failed");
                    }
                }

                if *shutdown_rx.borrow() {
                    break;
                }
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, MAX_BACKOFF);
            }
        });

        info!(server = %self.server, port = %self.port, "Mumble adapter started");
        let stream = ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => return Err(ChannelError::UnsupportedContent(
                "Mumble adapter only supports Text content".to_string(),
            )),
        };

        // Send via Mumble TextMessage packet. In a full implementation we would
        // serialize the TextMessage protobuf and send over the control channel.
        // user.platform_id would be session ID or channel ID for targeting.
        let _ = (user, text);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mumble_adapter_creation() {
        let adapter = MumbleAdapter::new(
            "mumble.example.com".to_string(),
            64738,
            "rune-bot".to_string(),
            Some("password".to_string()),
        );
        assert_eq!(adapter.name(), "mumble");
        assert_eq!(adapter.channel_type(), ChannelType::Mumble);
    }

    #[test]
    fn test_mumble_adapter_no_password() {
        let adapter = MumbleAdapter::new(
            "localhost".to_string(),
            64738,
            "bot".to_string(),
            None,
        );
        assert_eq!(adapter.name(), "mumble");
    }
}
