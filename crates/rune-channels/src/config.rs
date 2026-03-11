use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Output format for channel-specific message formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Markdown,
    TelegramHtml,
    SlackMrkdwn,
    PlainText,
}

impl Default for OutputFormat {
    fn default() -> Self {
        Self::Markdown
    }
}

/// Policy for handling DMs (direct messages).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DmPolicy {
    Respond,
    AllowedOnly,
    Ignore,
}

impl Default for DmPolicy {
    fn default() -> Self {
        Self::Respond
    }
}

/// Policy for handling group messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupPolicy {
    All,
    MentionOnly,
    CommandsOnly,
    Ignore,
}

impl Default for GroupPolicy {
    fn default() -> Self {
        Self::MentionOnly
    }
}

/// Per-channel configuration overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelOverrides {
    #[serde(default)]
    pub output_format: Option<OutputFormat>,
    #[serde(default)]
    pub dm_policy: DmPolicy,
    #[serde(default)]
    pub group_policy: GroupPolicy,
    #[serde(default)]
    pub threading: bool,
    #[serde(default)]
    pub rate_limit_per_user: u32,
}

impl Default for ChannelOverrides {
    fn default() -> Self {
        Self {
            output_format: None,
            dm_policy: DmPolicy::default(),
            group_policy: GroupPolicy::default(),
            threading: false,
            rate_limit_per_user: 0,
        }
    }
}

/// Rule for matching incoming messages to agents.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BindingMatchRule {
    pub channel: Option<String>,
    pub account_id: Option<String>,
    pub peer_id: Option<String>,
    pub guild_id: Option<String>,
    #[serde(default)]
    pub roles: Vec<String>,
}

impl BindingMatchRule {
    /// Compute specificity score: more fields = higher priority.
    /// peer_id: 8, guild_id: 4, roles: 2, account_id: 2, channel: 1.
    pub fn specificity(&self) -> u32 {
        let mut score = 0u32;
        if self.peer_id.is_some() {
            score += 8;
        }
        if self.guild_id.is_some() {
            score += 4;
        }
        if !self.roles.is_empty() {
            score += 2;
        }
        if self.account_id.is_some() {
            score += 2;
        }
        if self.channel.is_some() {
            score += 1;
        }
        score
    }
}

/// Binds an agent to a set of matching rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBinding {
    pub agent: String,
    pub match_rule: BindingMatchRule,
}

/// Strategy for broadcasting to multiple agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BroadcastStrategy {
    Parallel,
    Sequential,
}

impl Default for BroadcastStrategy {
    fn default() -> Self {
        Self::Parallel
    }
}

/// Configuration for broadcasting messages to multiple agents.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BroadcastConfig {
    #[serde(default)]
    pub strategy: BroadcastStrategy,
    #[serde(default)]
    pub routes: HashMap<String, Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binding_specificity() {
        let empty = BindingMatchRule::default();
        assert_eq!(empty.specificity(), 0);

        let channel_only = BindingMatchRule {
            channel: Some("discord".into()),
            ..Default::default()
        };
        assert_eq!(channel_only.specificity(), 1);

        let full = BindingMatchRule {
            channel: Some("discord".into()),
            peer_id: Some("user".into()),
            guild_id: Some("guild".into()),
            roles: vec!["admin".into()],
            account_id: Some("bot".into()),
        };
        assert_eq!(full.specificity(), 17);
    }

    #[test]
    fn test_output_format_serde() {
        let fmt = OutputFormat::TelegramHtml;
        let json = serde_json::to_string(&fmt).unwrap();
        let back: OutputFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(back, OutputFormat::TelegramHtml);
    }
}
