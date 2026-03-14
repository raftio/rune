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
        "telegram" => {
            let bot_token = param_str(p, "bot_token")?;
            let allowed_chat_ids = param_i64s(p, "allowed_chat_ids")?;
            Ok(Arc::new(crate::TelegramAdapter::new(bot_token, allowed_chat_ids)))
        }
        other => Err(ChannelError::Config(format!("Unknown channel type: '{other}'"))),
    }
}

/// Load channels from env vars (legacy approach, for backwards compatibility).
/// Checks for `RUNE_*` env vars and returns matching configs.
pub fn configs_from_env(env: &rune_env::PlatformEnv) -> Vec<ChannelConfig> {
    let mut configs = Vec::new();

    if let Some(token) = &env.telegram_bot_token {
        let mut params = HashMap::new();
        params.insert("bot_token".into(), serde_json::json!(token.as_str()));
        configs.push(ChannelConfig { channel_type: "telegram".into(), params });
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
  - type: telegram
    bot_token: "123:ABC"
"#;
        let file: ChannelsFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(file.channels.len(), 1);
        assert_eq!(file.channels[0].channel_type, "telegram");
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
            channel_type: "telegram".into(),
            params: HashMap::new(),
        };
        match create_adapter(&config) {
            Err(e) => assert!(e.to_string().contains("bot_token")),
            Ok(_) => panic!("Expected error for missing params"),
        }
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
