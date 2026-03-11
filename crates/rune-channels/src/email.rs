//! Email adapter (SMTP outbound + IMAP inbound).
//!
//! Sends emails via SMTP and receives emails by polling IMAP INBOX.

use crate::error::ChannelError;
use crate::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType, ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use mailparse::*;
use std::collections::HashMap;
use std::pin::Pin;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, warn};
use zeroize::Zeroizing;

/// Email adapter for SMTP outbound and IMAP inbound.
pub struct EmailAdapter {
    smtp_host: String,
    smtp_port: u16,
    smtp_user: Zeroizing<String>,
    smtp_pass: Zeroizing<String>,
    smtp_from: Zeroizing<String>,
    imap_host: String,
    imap_port: u16,
    imap_user: Zeroizing<String>,
    imap_pass: Zeroizing<String>,
    poll_interval_secs: u64,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl EmailAdapter {
    pub fn new(
        smtp_host: String,
        smtp_port: u16,
        smtp_user: String,
        smtp_pass: String,
        smtp_from: String,
        imap_host: String,
        imap_port: u16,
        imap_user: String,
        imap_pass: String,
        poll_interval_secs: u64,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            smtp_host,
            smtp_port,
            smtp_user: Zeroizing::new(smtp_user),
            smtp_pass: Zeroizing::new(smtp_pass),
            smtp_from: Zeroizing::new(smtp_from),
            imap_host,
            imap_port,
            imap_user: Zeroizing::new(imap_user),
            imap_pass: Zeroizing::new(imap_pass),
            poll_interval_secs,
            shutdown_tx,
            shutdown_rx,
        }
    }

    fn build_smtp_transport(&self) -> Result<AsyncSmtpTransport<Tokio1Executor>, ChannelError> {
        let creds = Credentials::new(
            self.smtp_user.as_str().to_string(),
            self.smtp_pass.as_str().to_string(),
        );
        let mailer = AsyncSmtpTransport::<Tokio1Executor>::relay(&self.smtp_host)
            .map_err(|e| ChannelError::Config(e.to_string()))?
            .port(self.smtp_port)
            .credentials(creds)
            .build();
        Ok(mailer)
    }
}

#[async_trait]
impl ChannelAdapter for EmailAdapter {
    fn name(&self) -> &str {
        "email"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Email
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(64);
        let imap_host = self.imap_host.clone();
        let imap_port = self.imap_port;
        let imap_user = self.imap_user.as_str().to_string();
        let imap_pass = self.imap_pass.as_str().to_string();
        let poll_interval_secs = self.poll_interval_secs;
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let result = tokio::task::spawn_blocking({
                    let imap_host = imap_host.clone();
                    let imap_user = imap_user.clone();
                    let imap_pass = imap_pass.clone();
                    let tx = tx.clone();

                    move || {
                        imap_poll_cycle(&imap_host, imap_port, &imap_user, &imap_pass, |msg| {
                            let _ = tx.blocking_send(msg);
                        })
                    }
                })
                .await;

                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        warn!("IMAP poll error: {}", e);
                    }
                    Err(e) => {
                        error!("IMAP poll task panicked: {}", e);
                    }
                }

                if *shutdown_rx.borrow() {
                    break;
                }

                tokio::select! {
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(poll_interval_secs)) => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
            }

            drop(tx);
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> {
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            ChannelContent::Image { url, caption } => {
                format!(
                    "{} {}",
                    url,
                    caption.as_deref().unwrap_or("")
                )
                .trim()
                .to_string()
            }
            ChannelContent::File { url, filename } => format!("[File: {}]({})", filename, url),
            _ => return Err(ChannelError::UnsupportedContent(format!("{:?}", content))),
        };

        let to_addr = user.platform_id.as_str();
        let from_addr = self.smtp_from.as_str();

        let email = Message::builder()
            .from(
                format!("Rune <{}>", from_addr)
                    .parse()
                    .map_err(|e: lettre::address::AddressError| ChannelError::Config(e.to_string()))?,
            )
            .to(to_addr
                .parse()
                .map_err(|e: lettre::address::AddressError| ChannelError::Config(e.to_string()))?)
            .subject("Message from Rune")
            .header(ContentType::TEXT_PLAIN)
            .body(text)
            .map_err(|e| ChannelError::Send(e.to_string()))?;

        let mailer = self.build_smtp_transport()?;
        mailer
            .send(email)
            .await
            .map_err(|e| ChannelError::Send(e.to_string()))?;

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

fn imap_poll_cycle<F>(
    imap_host: &str,
    imap_port: u16,
    imap_user: &str,
    imap_pass: &str,
    mut on_message: F,
) -> Result<(), ChannelError>
where
    F: FnMut(ChannelMessage),
{
    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| ChannelError::Connection(e.to_string()))?;

    let client = imap::connect((imap_host, imap_port), imap_host, &tls)
        .map_err(|e| ChannelError::Connection(e.to_string()))?;

    let mut session = client
        .login(imap_user, imap_pass)
        .map_err(|e| ChannelError::Auth(e.0.to_string()))?;

    session
        .select("INBOX")
        .map_err(|e| ChannelError::Connection(e.to_string()))?;

    let unseen = session
        .search("UNSEEN")
        .map_err(|e| ChannelError::Connection(e.to_string()))?;

    for seq in unseen.iter() {
        let fetch_result = session.fetch(seq.to_string(), "RFC822");
        if let Err(e) = fetch_result {
            warn!("IMAP fetch error: {}", e);
            continue;
        }

        for fetch in fetch_result.unwrap().iter() {
            let body = match fetch.body() {
                Some(b) => b,
                None => continue,
            };

            if let Ok(parsed) = parse_mail(body) {
                if let Some(msg) = parse_email_to_channel_message(&parsed, seq) {
                    on_message(msg);
                }

                session
                    .store(seq.to_string(), "+FLAGS (\\Seen)")
                    .map_err(|e| ChannelError::Connection(e.to_string()))?;
            }
        }
    }

    session
        .logout()
        .map_err(|e| ChannelError::Connection(e.to_string()))?;

    Ok(())
}

fn parse_email_to_channel_message(parsed: &ParsedMail, seq: impl std::fmt::Display) -> Option<ChannelMessage> {
    let from = parsed.headers.iter().find(|h| h.get_key() == "From")?;
    let from_val = from.get_value();
    let (email_addr, display_name) = parse_from_header(&from_val);

    let subject = parsed
        .headers
        .iter()
        .find(|h| h.get_key() == "Subject")
        .map(|h| h.get_value())
        .unwrap_or_default();

    let timestamp = parsed
        .headers
        .iter()
        .find(|h| h.get_key().eq_ignore_ascii_case("Date"))
        .and_then(|h| mailparse::dateparse(&h.get_value()).ok())
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let body = get_email_body(parsed);
    let platform_id = email_addr.clone();
    let mut metadata = HashMap::new();
    metadata.insert(
        "email_subject".to_string(),
        serde_json::Value::String(subject),
    );

    Some(ChannelMessage {
        channel: ChannelType::Email,
        platform_message_id: format!("imap:{}", seq),
        sender: ChannelUser {
            platform_id,
            display_name: display_name.unwrap_or_else(|| email_addr.clone()),
            rune_user: None,
        },
        content: ChannelContent::Text(body),
        target_agent: None,
        timestamp,
        is_group: false,
        thread_id: None,
        metadata,
    })
}

fn parse_from_header(from: &str) -> (String, Option<String>) {
    if let Some(lt) = from.find('<') {
        if let Some(gt) = from.find('>') {
            let addr = from[lt + 1..gt].trim().to_string();
            let name = from[..lt].trim().trim_matches('"').trim();
            let display = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            return (addr, display);
        }
    }
    (from.trim().to_string(), None)
}

fn get_email_body(parsed: &ParsedMail) -> String {
    if let Some(body) = parsed.subparts.first() {
        if let Ok(text) = body.get_body() {
            return text;
        }
    }
    parsed.get_body().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_adapter_new() {
        let adapter = EmailAdapter::new(
            "smtp.example.com".into(),
            587,
            "user".into(),
            "pass".into(),
            "from@example.com".into(),
            "imap.example.com".into(),
            993,
            "user".into(),
            "pass".into(),
            60,
        );
        assert_eq!(adapter.name(), "email");
        assert_eq!(adapter.channel_type(), ChannelType::Email);
    }

    #[test]
    fn test_parse_from_header() {
        let (addr, name) = parse_from_header("Alice <alice@example.com>");
        assert_eq!(addr, "alice@example.com");
        assert_eq!(name, Some("Alice".to_string()));

        let (addr2, name2) = parse_from_header("bob@example.com");
        assert_eq!(addr2, "bob@example.com");
        assert_eq!(name2, None);
    }
}
