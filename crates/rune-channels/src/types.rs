use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;

use crate::error::ChannelError;

/// The type of messaging channel.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChannelType {
    Telegram,
    WhatsApp,
    Slack,
    Discord,
    Signal,
    Matrix,
    Email,
    Teams,
    Mattermost,
    GoogleChat,
    Irc,
    RocketChat,
    Xmpp,
    Zulip,
    Twitch,
    Line,
    Messenger,
    Viber,
    Reddit,
    Mastodon,
    Bluesky,
    Feishu,
    Revolt,
    Webex,
    Flock,
    Guilded,
    Keybase,
    Nextcloud,
    Nostr,
    Pumble,
    Threema,
    Twist,
    DingTalk,
    Discourse,
    Gitter,
    Gotify,
    LinkedIn,
    Mumble,
    Ntfy,
    WebChat,
    CLI,
    Custom(String),
}

impl ChannelType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Telegram => "telegram",
            Self::WhatsApp => "whatsapp",
            Self::Slack => "slack",
            Self::Discord => "discord",
            Self::Signal => "signal",
            Self::Matrix => "matrix",
            Self::Email => "email",
            Self::Teams => "teams",
            Self::Mattermost => "mattermost",
            Self::GoogleChat => "google_chat",
            Self::Irc => "irc",
            Self::RocketChat => "rocketchat",
            Self::Xmpp => "xmpp",
            Self::Zulip => "zulip",
            Self::Twitch => "twitch",
            Self::Line => "line",
            Self::Messenger => "messenger",
            Self::Viber => "viber",
            Self::Reddit => "reddit",
            Self::Mastodon => "mastodon",
            Self::Bluesky => "bluesky",
            Self::Feishu => "feishu",
            Self::Revolt => "revolt",
            Self::Webex => "webex",
            Self::Flock => "flock",
            Self::Guilded => "guilded",
            Self::Keybase => "keybase",
            Self::Nextcloud => "nextcloud",
            Self::Nostr => "nostr",
            Self::Pumble => "pumble",
            Self::Threema => "threema",
            Self::Twist => "twist",
            Self::DingTalk => "dingtalk",
            Self::Discourse => "discourse",
            Self::Gitter => "gitter",
            Self::Gotify => "gotify",
            Self::LinkedIn => "linkedin",
            Self::Mumble => "mumble",
            Self::Ntfy => "ntfy",
            Self::WebChat => "webchat",
            Self::CLI => "cli",
            Self::Custom(s) => s.as_str(),
        }
    }
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A user on a messaging platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelUser {
    /// Platform-specific user ID.
    pub platform_id: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Optional mapping to a Rune user/tenant identity.
    pub rune_user: Option<String>,
}

/// Content types that can be received from or sent to a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChannelContent {
    Text(String),
    Image {
        url: String,
        caption: Option<String>,
    },
    File {
        url: String,
        filename: String,
    },
    Voice {
        url: String,
        duration_seconds: u32,
    },
    Location {
        lat: f64,
        lon: f64,
    },
    Command {
        name: String,
        args: Vec<String>,
    },
}

/// A unified message from any channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    pub channel: ChannelType,
    pub platform_message_id: String,
    pub sender: ChannelUser,
    pub content: ChannelContent,
    pub target_agent: Option<uuid::Uuid>,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub is_group: bool,
    #[serde(default)]
    pub thread_id: Option<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Agent lifecycle phase for UX indicators on channels.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AgentPhase {
    Queued,
    Thinking,
    ToolUse { tool_name: String },
    Streaming,
    Done,
    Error,
}

impl AgentPhase {
    pub fn tool_use(name: &str) -> Self {
        let sanitized: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
        Self::ToolUse {
            tool_name: sanitized,
        }
    }
}

/// Emoji reaction representing an agent lifecycle phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleReaction {
    pub phase: AgentPhase,
    pub emoji: String,
    pub remove_previous: bool,
}

pub const ALLOWED_REACTION_EMOJI: &[&str] = &[
    "\u{1F914}", // 🤔 thinking
    "\u{2699}\u{FE0F}", // ⚙️ tool_use
    "\u{270D}\u{FE0F}", // ✍️ streaming
    "\u{2705}", // ✅ done
    "\u{274C}", // ❌ error
    "\u{23F3}", // ⏳ queued
    "\u{1F504}", // 🔄 processing
    "\u{1F440}", // 👀 looking
];

pub fn default_phase_emoji(phase: &AgentPhase) -> &'static str {
    match phase {
        AgentPhase::Queued => "\u{23F3}",
        AgentPhase::Thinking => "\u{1F914}",
        AgentPhase::ToolUse { .. } => "\u{2699}\u{FE0F}",
        AgentPhase::Streaming => "\u{270D}\u{FE0F}",
        AgentPhase::Done => "\u{2705}",
        AgentPhase::Error => "\u{274C}",
    }
}

/// Delivery status for outbound messages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Sent,
    Delivered,
    Failed,
    BestEffort,
}

/// Receipt tracking for outbound message delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryReceipt {
    pub message_id: String,
    pub channel: String,
    pub recipient: String,
    pub status: DeliveryStatus,
    pub timestamp: DateTime<Utc>,
    pub error: Option<String>,
}

/// Health status for a channel adapter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelStatus {
    pub connected: bool,
    pub started_at: Option<DateTime<Utc>>,
    pub last_message_at: Option<DateTime<Utc>>,
    pub messages_received: u64,
    pub messages_sent: u64,
    pub last_error: Option<String>,
}

/// Trait that every channel adapter must implement.
///
/// Implement this to bridge any messaging platform to Rune. The adapter
/// converts platform-specific messages into `ChannelMessage` events and
/// sends responses back.
///
/// # Example
///
/// ```ignore
/// struct MyAdapter { /* ... */ }
///
/// #[async_trait]
/// impl ChannelAdapter for MyAdapter {
///     fn name(&self) -> &str { "my-adapter" }
///     fn channel_type(&self) -> ChannelType { ChannelType::Custom("my-platform".into()) }
///     async fn start(&self) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError> { /* ... */ }
///     async fn send(&self, user: &ChannelUser, content: ChannelContent) -> Result<(), ChannelError> { /* ... */ }
///     async fn stop(&self) -> Result<(), ChannelError> { /* ... */ }
/// }
/// ```
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Human-readable name of this adapter.
    fn name(&self) -> &str;

    /// The channel type this adapter handles.
    fn channel_type(&self) -> ChannelType;

    /// Start receiving messages. Returns a stream of incoming messages.
    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, ChannelError>;

    /// Send a response back to a user on this channel.
    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), ChannelError>;

    /// Send a typing indicator (optional).
    async fn send_typing(&self, _user: &ChannelUser) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Send a lifecycle reaction to a message (optional).
    async fn send_reaction(
        &self,
        _user: &ChannelUser,
        _message_id: &str,
        _reaction: &LifecycleReaction,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Stop the adapter and clean up resources.
    async fn stop(&self) -> Result<(), ChannelError>;

    /// Get the current health status of this adapter.
    fn status(&self) -> ChannelStatus {
        ChannelStatus::default()
    }

    /// Send a response as a thread reply (falls back to `send()`).
    async fn send_in_thread(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
        _thread_id: &str,
    ) -> Result<(), ChannelError> {
        self.send(user, content).await
    }
}

/// Split a message into chunks of at most `max_len` characters,
/// preferring to split at newline boundaries.
pub fn split_message(text: &str, max_len: usize) -> Vec<&str> {
    if text.len() <= max_len {
        return vec![text];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining);
            break;
        }
        let safe_end = remaining
            .char_indices()
            .take_while(|(i, _)| *i <= max_len)
            .last()
            .map(|(i, _)| i)
            .unwrap_or(max_len);
        let split_at = remaining[..safe_end].rfind('\n').unwrap_or(safe_end);
        let (chunk, rest) = remaining.split_at(split_at);
        chunks.push(chunk);
        remaining = rest
            .strip_prefix("\r\n")
            .or_else(|| rest.strip_prefix('\n'))
            .unwrap_or(rest);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_message_serialization() {
        let msg = ChannelMessage {
            channel: ChannelType::Telegram,
            platform_message_id: "123".to_string(),
            sender: ChannelUser {
                platform_id: "user1".to_string(),
                display_name: "Alice".to_string(),
                rune_user: None,
            },
            content: ChannelContent::Text("Hello!".to_string()),
            target_agent: None,
            timestamp: Utc::now(),
            is_group: false,
            thread_id: None,
            metadata: HashMap::new(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ChannelMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.channel, ChannelType::Telegram);
    }

    #[test]
    fn test_split_message_short() {
        assert_eq!(split_message("hello", 100), vec!["hello"]);
    }

    #[test]
    fn test_split_message_at_newlines() {
        let text = "line1\nline2\nline3";
        let chunks = split_message(text, 10);
        assert_eq!(chunks, vec!["line1", "line2", "line3"]);
    }

    #[test]
    fn test_channel_type_serde() {
        let ct = ChannelType::Matrix;
        let json = serde_json::to_string(&ct).unwrap();
        let back: ChannelType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ChannelType::Matrix);
    }

    #[test]
    fn test_agent_phase_serde_roundtrip() {
        let phases = vec![
            AgentPhase::Queued,
            AgentPhase::Thinking,
            AgentPhase::tool_use("web_fetch"),
            AgentPhase::Streaming,
            AgentPhase::Done,
            AgentPhase::Error,
        ];
        for phase in &phases {
            let json = serde_json::to_string(phase).unwrap();
            let back: AgentPhase = serde_json::from_str(&json).unwrap();
            assert_eq!(*phase, back);
        }
    }

    #[test]
    fn test_agent_phase_tool_use_sanitizes() {
        let phase = AgentPhase::tool_use("hello\x00world\x01test");
        if let AgentPhase::ToolUse { tool_name } = phase {
            assert!(!tool_name.contains('\x00'));
            assert!(tool_name.contains("hello"));
        } else {
            panic!("Expected ToolUse variant");
        }
    }

    #[test]
    fn test_delivery_status_serde() {
        for status in &[
            DeliveryStatus::Sent,
            DeliveryStatus::Delivered,
            DeliveryStatus::Failed,
            DeliveryStatus::BestEffort,
        ] {
            let json = serde_json::to_string(status).unwrap();
            let back: DeliveryStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(*status, back);
        }
    }

    #[test]
    fn test_default_phase_emoji() {
        assert_eq!(default_phase_emoji(&AgentPhase::Thinking), "\u{1F914}");
        assert_eq!(default_phase_emoji(&AgentPhase::Done), "\u{2705}");
        assert_eq!(default_phase_emoji(&AgentPhase::Error), "\u{274C}");
    }
}
