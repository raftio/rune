use crate::error::ChannelError;
use crate::types::ChannelAdapter;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Declarative channel adapter configuration.
///
/// Can be loaded from YAML, JSON, or env vars. Each entry
/// describes which adapter to start and its parameters.
///
/// # Example (YAML)
///
/// ```yaml
/// channels:
///   - type: slack
///     app_token: ${RUNE_SLACK_APP_TOKEN}
///     bot_token: ${RUNE_SLACK_BOT_TOKEN}
///   - type: telegram
///     bot_token: ${RUNE_TELEGRAM_BOT_TOKEN}
///   - type: webhook
///     secret: my-secret
///     listen_port: 9090
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    #[serde(rename = "type")]
    pub channel_type: String,
    #[serde(flatten)]
    pub params: HashMap<String, serde_json::Value>,
}

/// Top-level channels configuration file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelsFile {
    #[serde(default)]
    pub channels: Vec<ChannelConfig>,
}

fn param_str(params: &HashMap<String, serde_json::Value>, key: &str) -> Result<String, ChannelError> {
    let raw = params
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChannelError::Config(format!("Missing required parameter '{key}'")))?;
    expand_env(raw)
}

fn param_str_opt(params: &HashMap<String, serde_json::Value>, key: &str) -> Result<Option<String>, ChannelError> {
    match params.get(key).and_then(|v| v.as_str()) {
        Some(s) => Ok(Some(expand_env(s)?)),
        None => Ok(None),
    }
}

fn param_u16(params: &HashMap<String, serde_json::Value>, key: &str, default: u16) -> Result<u16, ChannelError> {
    match params.get(key) {
        Some(v) => {
            if let Some(n) = v.as_u64() {
                return Ok(n as u16);
            }
            if let Some(s) = v.as_str() {
                let expanded = expand_env(s)?;
                return expanded.parse().map_err(|_| {
                    ChannelError::Config(format!("Parameter '{key}' is not a valid u16: '{expanded}'"))
                });
            }
            Ok(default)
        }
        None => Ok(default),
    }
}

fn param_u64(params: &HashMap<String, serde_json::Value>, key: &str, default: u64) -> Result<u64, ChannelError> {
    match params.get(key) {
        Some(v) => {
            if let Some(n) = v.as_u64() {
                return Ok(n);
            }
            if let Some(s) = v.as_str() {
                let expanded = expand_env(s)?;
                return expanded.parse().map_err(|_| {
                    ChannelError::Config(format!("Parameter '{key}' is not a valid u64: '{expanded}'"))
                });
            }
            Ok(default)
        }
        None => Ok(default),
    }
}

fn param_bool(params: &HashMap<String, serde_json::Value>, key: &str, default: bool) -> bool {
    params
        .get(key)
        .and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "true" || s == "1")))
        .unwrap_or(default)
}

fn param_strings(params: &HashMap<String, serde_json::Value>, key: &str) -> Result<Vec<String>, ChannelError> {
    match params.get(key) {
        Some(v) => {
            if let Some(arr) = v.as_array() {
                let mut result = Vec::new();
                for e in arr {
                    if let Some(s) = e.as_str() {
                        result.push(expand_env(s)?);
                    }
                }
                Ok(result)
            } else if let Some(s) = v.as_str() {
                let expanded = expand_env(s)?;
                Ok(expanded.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            } else {
                Ok(Vec::new())
            }
        }
        None => Ok(Vec::new()),
    }
}

fn param_i64s(params: &HashMap<String, serde_json::Value>, key: &str) -> Result<Vec<i64>, ChannelError> {
    match params.get(key) {
        Some(v) => {
            if let Some(arr) = v.as_array() {
                Ok(arr.iter().filter_map(|e| e.as_i64()).collect())
            } else if let Some(s) = v.as_str() {
                let expanded = expand_env(s)?;
                Ok(expanded.split(',').filter_map(|s| s.trim().parse().ok()).collect())
            } else {
                Ok(Vec::new())
            }
        }
        None => Ok(Vec::new()),
    }
}

/// Expand `${VAR}` references in a string.
///
/// Returns an error if a referenced env var does not exist,
/// failing fast on misconfiguration rather than silently passing empty strings.
fn expand_env(s: &str) -> Result<String, ChannelError> {
    let mut result = s.to_string();
    while let Some(start) = result.find("${") {
        if let Some(end) = result[start..].find('}') {
            let end = start + end;
            let var_name = &result[start + 2..end];
            let value = std::env::var(var_name).map_err(|_| {
                ChannelError::Config(format!("Env var ${{{var_name}}} is not set"))
            })?;
            result = format!("{}{}{}", &result[..start], value, &result[end + 1..]);
        } else {
            break;
        }
    }
    Ok(result)
}

/// Create an adapter from a declarative config entry.
pub fn create_adapter(config: &ChannelConfig) -> Result<Arc<dyn ChannelAdapter>, ChannelError> {
    let p = &config.params;
    match config.channel_type.as_str() {
        "slack" => {
            let app_token = param_str(p, "app_token")?;
            let bot_token = param_str(p, "bot_token")?;
            let allowed_channels = param_strings(p, "allowed_channels")?;
            Ok(Arc::new(crate::SlackAdapter::new(app_token, bot_token, allowed_channels)))
        }
        "telegram" => {
            let bot_token = param_str(p, "bot_token")?;
            let allowed_chat_ids = param_i64s(p, "allowed_chat_ids")?;
            Ok(Arc::new(crate::TelegramAdapter::new(bot_token, allowed_chat_ids)))
        }
        "discord" => {
            let bot_token = param_str(p, "bot_token")?;
            let allowed_guild_ids = param_strings(p, "allowed_guild_ids")?;
            let allowed_channel_ids = param_strings(p, "allowed_channel_ids")?;
            Ok(Arc::new(crate::DiscordAdapter::new(bot_token, allowed_guild_ids, allowed_channel_ids)))
        }
        "email" => {
            Ok(Arc::new(crate::EmailAdapter::new(
                param_str(p, "smtp_host")?,
                param_u16(p, "smtp_port", 587)?,
                param_str(p, "smtp_user")?,
                param_str(p, "smtp_pass")?,
                param_str(p, "smtp_from")?,
                param_str(p, "imap_host")?,
                param_u16(p, "imap_port", 993)?,
                param_str(p, "imap_user")?,
                param_str(p, "imap_pass")?,
                param_u64(p, "poll_interval_secs", 30)?,
            )))
        }
        "webhook" => {
            let secret = param_str(p, "secret")?;
            let listen_port = param_u16(p, "listen_port", 9090)?;
            let callback_url = param_str_opt(p, "callback_url")?;
            Ok(Arc::new(crate::WebhookAdapter::new(secret, listen_port, callback_url)?))
        }
        "teams" => {
            Ok(Arc::new(crate::TeamsAdapter::new(
                param_str(p, "app_id")?,
                param_str(p, "app_password")?,
                param_str(p, "service_url")?,
            )))
        }
        "whatsapp" => {
            Ok(Arc::new(crate::WhatsAppAdapter::new(
                param_str(p, "phone_number_id")?,
                param_str(p, "access_token")?,
                param_str(p, "verify_token")?,
                param_u16(p, "webhook_port", 8443)?,
            )))
        }
        "matrix" => {
            Ok(Arc::new(crate::MatrixAdapter::new(
                param_str(p, "homeserver_url")?,
                param_str(p, "access_token")?,
                param_str(p, "user_id")?,
            )))
        }
        "mattermost" => {
            Ok(Arc::new(crate::MattermostAdapter::new(
                param_str(p, "server_url")?,
                param_str(p, "token")?,
            )))
        }
        "signal" => {
            Ok(Arc::new(crate::SignalAdapter::new(
                param_str(p, "api_url")?,
                param_str(p, "phone_number")?,
            )))
        }
        "google_chat" => {
            Ok(Arc::new(crate::GoogleChatAdapter::new(
                param_str(p, "project_id")?,
                param_str(p, "credentials_json")?,
                param_u16(p, "webhook_port", 8080)?,
            )))
        }
        "irc" => {
            Ok(Arc::new(crate::IrcAdapter::new(
                param_str(p, "server")?,
                param_u16(p, "port", 6697)?,
                param_str(p, "nick")?,
                param_strings(p, "channels")?,
                param_str_opt(p, "password")?,
                param_bool(p, "use_tls", true),
            )))
        }
        "rocketchat" => {
            Ok(Arc::new(crate::RocketChatAdapter::new(
                param_str(p, "server_url")?,
                param_str(p, "user_id")?,
                param_str(p, "auth_token")?,
            )))
        }
        "xmpp" => {
            Ok(Arc::new(crate::XmppAdapter::new(
                param_str(p, "jid")?,
                param_str(p, "password")?,
                param_str(p, "server")?,
            )))
        }
        "zulip" => {
            Ok(Arc::new(crate::ZulipAdapter::new(
                param_str(p, "server_url")?,
                param_str(p, "email")?,
                param_str(p, "api_key")?,
            )))
        }
        "twitch" => {
            Ok(Arc::new(crate::TwitchAdapter::new(
                param_str(p, "oauth_token")?,
                param_str(p, "nick")?,
                param_strings(p, "channels")?,
            )))
        }
        "line" => {
            Ok(Arc::new(crate::LineAdapter::new(
                param_str(p, "channel_secret")?,
                param_str(p, "channel_access_token")?,
                param_u16(p, "webhook_port", 8443)?,
            )))
        }
        "messenger" => {
            Ok(Arc::new(crate::MessengerAdapter::new(
                param_str(p, "page_access_token")?,
                param_str(p, "verify_token")?,
                param_str(p, "app_secret")?,
                param_u16(p, "webhook_port", 8443)?,
            )))
        }
        "viber" => {
            Ok(Arc::new(crate::ViberAdapter::new(
                param_str(p, "auth_token")?,
                param_str(p, "webhook_url")?,
                param_u16(p, "webhook_port", 8443)?,
            )))
        }
        "reddit" => {
            Ok(Arc::new(crate::RedditAdapter::new(
                param_str(p, "client_id")?,
                param_str(p, "client_secret")?,
                param_str(p, "username")?,
                param_str(p, "password")?,
                param_strings(p, "subreddits")?,
            )))
        }
        "mastodon" => {
            Ok(Arc::new(crate::MastodonAdapter::new(
                param_str(p, "instance_url")?,
                param_str(p, "access_token")?,
            )))
        }
        "bluesky" => {
            Ok(Arc::new(crate::BlueskyAdapter::new(
                param_str(p, "handle")?,
                param_str(p, "app_password")?,
                param_str(p, "pds_url")?,
            )))
        }
        "feishu" => {
            Ok(Arc::new(crate::FeishuAdapter::new(
                param_str(p, "app_id")?,
                param_str(p, "app_secret")?,
                param_u16(p, "webhook_port", 8080)?,
            )))
        }
        "revolt" => {
            Ok(Arc::new(crate::RevoltAdapter::new(
                param_str(p, "token")?,
                param_str(p, "api_url")?,
            )))
        }
        "webex" => {
            Ok(Arc::new(crate::WebexAdapter::new(
                param_str(p, "access_token")?,
                param_str_opt(p, "webhook_url")?,
                param_u16(p, "webhook_port", 8080)?,
            )))
        }
        "flock" => {
            Ok(Arc::new(crate::FlockAdapter::new(
                param_str(p, "bot_token")?,
                param_u16(p, "webhook_port", 8080)?,
            )))
        }
        "guilded" => {
            Ok(Arc::new(crate::GuildedAdapter::new(
                param_str(p, "token")?,
                param_str_opt(p, "api_url")?,
            )))
        }
        "keybase" => {
            Ok(Arc::new(crate::KeybaseAdapter::new(
                param_str(p, "username")?,
                param_str(p, "paper_key")?,
            )))
        }
        "nextcloud" => {
            Ok(Arc::new(crate::NextcloudAdapter::new(
                param_str(p, "server_url")?,
                param_str(p, "username")?,
                param_str(p, "password")?,
                param_str(p, "room_token")?,
            )))
        }
        "nostr" => {
            Ok(Arc::new(crate::NostrAdapter::new(
                param_str(p, "private_key")?,
                param_strings(p, "relays")?,
            )))
        }
        "pumble" => {
            Ok(Arc::new(crate::PumbleAdapter::new(
                param_str(p, "api_token")?,
                param_u16(p, "webhook_port", 8080)?,
            )))
        }
        "threema" => {
            Ok(Arc::new(crate::ThreemaAdapter::new(
                param_str(p, "api_identity")?,
                param_str(p, "api_secret")?,
                param_u16(p, "webhook_port", 8080)?,
            )))
        }
        "twist" => {
            Ok(Arc::new(crate::TwistAdapter::new(
                param_str(p, "oauth_token")?,
                param_str(p, "workspace_id")?,
            )))
        }
        "dingtalk" => {
            Ok(Arc::new(crate::DingTalkAdapter::new(
                param_str(p, "app_key")?,
                param_str(p, "app_secret")?,
                param_u16(p, "webhook_port", 8080)?,
            )))
        }
        "discourse" => {
            Ok(Arc::new(crate::DiscourseAdapter::new(
                param_str(p, "base_url")?,
                param_str(p, "api_key")?,
                param_str(p, "api_username")?,
                param_u16(p, "webhook_port", 8080)?,
            )))
        }
        "gitter" => {
            Ok(Arc::new(crate::GitterAdapter::new(
                param_str(p, "token")?,
                param_str(p, "room_id")?,
            )))
        }
        "gotify" => {
            Ok(Arc::new(crate::GotifyAdapter::new(
                param_str(p, "server_url")?,
                param_str(p, "app_token")?,
                param_str(p, "client_token")?,
            )))
        }
        "linkedin" => {
            Ok(Arc::new(crate::LinkedInAdapter::new(
                param_str(p, "access_token")?,
                param_u16(p, "webhook_port", 8080)?,
            )))
        }
        "mumble" => {
            Ok(Arc::new(crate::MumbleAdapter::new(
                param_str(p, "server")?,
                param_u16(p, "port", 64738)?,
                param_str(p, "username")?,
                param_str_opt(p, "password")?,
            )))
        }
        "ntfy" => {
            Ok(Arc::new(crate::NtfyAdapter::new(
                param_str(p, "topic")?,
                param_str_opt(p, "server_url")?,
                param_str_opt(p, "auth_token")?,
            )))
        }
        other => Err(ChannelError::Config(format!("Unknown channel type: '{other}'"))),
    }
}

/// Load channels from env vars (legacy approach, for backwards compatibility).
/// Checks for `RUNE_*` env vars and returns matching configs.
pub fn configs_from_env(env: &rune_env::PlatformEnv) -> Vec<ChannelConfig> {
    let mut configs = Vec::new();

    if env.slack_app_token.is_some() && env.slack_bot_token.is_some() {
        let mut params = HashMap::new();
        params.insert("app_token".into(), serde_json::json!("${RUNE_SLACK_APP_TOKEN}"));
        params.insert("bot_token".into(), serde_json::json!("${RUNE_SLACK_BOT_TOKEN}"));
        configs.push(ChannelConfig { channel_type: "slack".into(), params });
    }

    if env.telegram_bot_token.is_some() {
        let mut params = HashMap::new();
        params.insert("bot_token".into(), serde_json::json!("${RUNE_TELEGRAM_BOT_TOKEN}"));
        configs.push(ChannelConfig { channel_type: "telegram".into(), params });
    }

    if env.discord_bot_token.is_some() {
        let mut params = HashMap::new();
        params.insert("bot_token".into(), serde_json::json!("${RUNE_DISCORD_BOT_TOKEN}"));
        configs.push(ChannelConfig { channel_type: "discord".into(), params });
    }

    if env.webhook_secret.is_some() {
        let mut params = HashMap::new();
        params.insert("secret".into(), serde_json::json!("${RUNE_WEBHOOK_SECRET}"));
        if let Some(port) = &env.webhook_port {
            params.insert("listen_port".into(), serde_json::json!(port));
        }
        if let Some(url) = &env.webhook_callback_url {
            params.insert("callback_url".into(), serde_json::json!(url));
        }
        configs.push(ChannelConfig { channel_type: "webhook".into(), params });
    }

    if env.matrix_access_token.is_some() {
        let mut params = HashMap::new();
        params.insert("homeserver_url".into(), serde_json::json!("${RUNE_MATRIX_HOMESERVER}"));
        params.insert("access_token".into(), serde_json::json!("${RUNE_MATRIX_ACCESS_TOKEN}"));
        params.insert("user_id".into(), serde_json::json!("${RUNE_MATRIX_USER_ID}"));
        configs.push(ChannelConfig { channel_type: "matrix".into(), params });
    }

    if env.mattermost_token.is_some() {
        let mut params = HashMap::new();
        params.insert("server_url".into(), serde_json::json!("${RUNE_MATTERMOST_SERVER_URL}"));
        params.insert("token".into(), serde_json::json!("${RUNE_MATTERMOST_TOKEN}"));
        configs.push(ChannelConfig { channel_type: "mattermost".into(), params });
    }

    if env.whatsapp_access_token.is_some() {
        let mut params = HashMap::new();
        params.insert("phone_number_id".into(), serde_json::json!("${RUNE_WHATSAPP_PHONE_NUMBER_ID}"));
        params.insert("access_token".into(), serde_json::json!("${RUNE_WHATSAPP_ACCESS_TOKEN}"));
        params.insert("verify_token".into(), serde_json::json!("${RUNE_WHATSAPP_VERIFY_TOKEN}"));
        configs.push(ChannelConfig { channel_type: "whatsapp".into(), params });
    }

    configs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_env() {
        std::env::set_var("TEST_RUNE_VAR", "hello");
        assert_eq!(expand_env("prefix_${TEST_RUNE_VAR}_suffix").unwrap(), "prefix_hello_suffix");
        assert_eq!(expand_env("no_vars").unwrap(), "no_vars");
        std::env::remove_var("TEST_RUNE_VAR");
    }

    #[test]
    fn test_expand_env_missing_var() {
        let result = expand_env("${RUNE_NONEXISTENT_TEST_VAR_XYZZY}");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("RUNE_NONEXISTENT_TEST_VAR_XYZZY"));
    }

    #[test]
    fn test_channel_config_serde() {
        let yaml = r#"
channels:
  - type: slack
    app_token: xapp-123
    bot_token: xoxb-456
  - type: telegram
    bot_token: "123:ABC"
  - type: webhook
    secret: my-secret
    listen_port: 9090
"#;
        let file: ChannelsFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(file.channels.len(), 3);
        assert_eq!(file.channels[0].channel_type, "slack");
        assert_eq!(file.channels[1].channel_type, "telegram");
        assert_eq!(file.channels[2].channel_type, "webhook");
    }

    #[test]
    fn test_create_adapter_unknown_type() {
        let config = ChannelConfig {
            channel_type: "foobar".into(),
            params: HashMap::new(),
        };
        assert!(create_adapter(&config).is_err());
    }

    #[test]
    fn test_create_adapter_missing_param() {
        let config = ChannelConfig {
            channel_type: "slack".into(),
            params: HashMap::new(),
        };
        match create_adapter(&config) {
            Err(e) => assert!(e.to_string().contains("app_token")),
            Ok(_) => panic!("Expected error for missing params"),
        }
    }

    #[test]
    fn test_create_adapter_slack() {
        let mut params = HashMap::new();
        params.insert("app_token".into(), serde_json::json!("xapp-test"));
        params.insert("bot_token".into(), serde_json::json!("xoxb-test"));
        let config = ChannelConfig {
            channel_type: "slack".into(),
            params,
        };
        let adapter = create_adapter(&config).unwrap();
        assert_eq!(adapter.name(), "slack");
    }

    #[test]
    fn test_create_adapter_telegram() {
        let mut params = HashMap::new();
        params.insert("bot_token".into(), serde_json::json!("123:ABC"));
        let config = ChannelConfig {
            channel_type: "telegram".into(),
            params,
        };
        let adapter = create_adapter(&config).unwrap();
        assert_eq!(adapter.name(), "telegram");
    }

    #[test]
    fn test_param_strings_csv() {
        let mut params = HashMap::new();
        params.insert("channels".into(), serde_json::json!("general, random, dev"));
        let result = param_strings(&params, "channels").unwrap();
        assert_eq!(result, vec!["general", "random", "dev"]);
    }

    #[test]
    fn test_param_strings_array() {
        let mut params = HashMap::new();
        params.insert("channels".into(), serde_json::json!(["general", "random"]));
        let result = param_strings(&params, "channels").unwrap();
        assert_eq!(result, vec!["general", "random"]);
    }
}
