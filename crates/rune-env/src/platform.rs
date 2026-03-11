use std::collections::HashMap;

use crate::error::EnvError;
use zeroize::Zeroizing;

/// Default path for the rune config file: `~/.rune/config.toml`.
pub fn default_config_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".rune")
        .join("config.toml")
}

const CONFIG_TEMPLATE: &str = r#"# Rune configuration file — ~/.rune/config.toml
# Keys match the corresponding environment variable names.

# ── LLM providers ────────────────────────────────────────────────────────────
# ANTHROPIC_API_KEY = ""
# OPENAI_API_KEY    = ""
# GEMINI_API_KEY    = ""

# ── Telegram ─────────────────────────────────────────────────────────────────
# RUNE_TELEGRAM_BOT_TOKEN = ""

# ── Slack ────────────────────────────────────────────────────────────────────
# RUNE_SLACK_BOT_TOKEN = ""
# RUNE_SLACK_APP_TOKEN = ""

# ── Discord ───────────────────────────────────────────────────────────────────
# RUNE_DISCORD_BOT_TOKEN = ""

# ── SMTP (email) ──────────────────────────────────────────────────────────────
# RUNE_SMTP_HOST = ""
# RUNE_SMTP_USER = ""
# RUNE_SMTP_PASS = ""
# RUNE_SMTP_FROM = ""

# ── Gateway ───────────────────────────────────────────────────────────────────
# GATEWAY_BASE_URL = "http://localhost:3000"
"#;

/// Load `~/.rune/config.toml` into a key-value map.
/// Creates the file with a commented template if it does not exist.
pub fn load_config_file() -> HashMap<String, String> {
    let path = default_config_path();

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if !path.exists() {
        let _ = std::fs::write(&path, CONFIG_TEMPLATE);
        return HashMap::new();
    }

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(path = %path.display(), "failed to read config file: {e}");
            return HashMap::new();
        }
    };

    let table = match toml::from_str::<toml::Table>(&content) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(path = %path.display(), "failed to parse config file: {e}");
            return HashMap::new();
        }
    };

    let map: HashMap<String, String> = table
        .into_iter()
        .filter_map(|(k, v)| {
            if let toml::Value::String(s) = v {
                if !s.is_empty() { Some((k, s)) } else { None }
            } else {
                None
            }
        })
        .collect();

    // Populate env vars so that code which reads std::env::var() directly also works.
    // SAFETY: called at startup before multi-threaded runtime.
    for (k, v) in &map {
        if std::env::var(k).unwrap_or_default().is_empty() {
            #[allow(deprecated)]
            unsafe { std::env::set_var(k, v) };
        }
    }

    tracing::debug!(path = %path.display(), keys = map.len(), "loaded config file");
    map
}

fn val<'a>(config: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    config.get(name).map(|s| s.as_str()).filter(|s| !s.is_empty())
}

fn val_or<'a>(config: &'a HashMap<String, String>, name: &str, default: &'a str) -> &'a str {
    val(config, name).unwrap_or(default)
}

fn parse_val<T: std::str::FromStr>(
    config: &HashMap<String, String>,
    name: &str,
    default: T,
) -> Result<T, EnvError> {
    match val(config, name) {
        Some(v) => v.parse().map_err(|_| EnvError::Invalid {
            var: name.to_string(),
            reason: format!("cannot parse '{v}'"),
        }),
        None => Ok(default),
    }
}

/// Centralized platform-level configuration.
///
/// Initialized once at startup from `~/.rune/config.toml` and shared via
/// `Arc<PlatformEnv>` across all crates.
#[derive(Debug, Clone)]
pub struct PlatformEnv {
    // -- Gateway --
    pub gateway_base_url: String,
    pub agent_workspace_dir: Option<String>,
    pub agent_packages_dir: Option<String>,

    // -- Auth --
    pub api_key: Option<Zeroizing<String>>,
    pub jwt_secret: Option<Zeroizing<String>>,

    // -- Rate limiting --
    pub rate_limit_rps: f64,
    pub rate_limit_burst: u32,

    // -- LLM providers --
    pub openai_api_key: Option<Zeroizing<String>>,
    pub openai_model: String,
    pub openai_base_url: String,
    pub anthropic_api_key: Option<Zeroizing<String>>,
    pub claude_code_api_key: Option<Zeroizing<String>>,
    pub gemini_api_key: Option<Zeroizing<String>>,
    pub gemini_model: String,
    pub copilot_api_key: Option<Zeroizing<String>>,

    // -- Storage --
    pub object_store_url: Option<String>,
    pub checkpoint_storage_threshold: usize,

    // -- Channels --
    pub channels_config_path: Option<String>,
    pub channel_agent: Option<String>,
    pub webhook_secret: Option<String>,
    pub webhook_port: Option<String>,
    pub webhook_callback_url: Option<String>,

    pub slack_app_token: Option<String>,
    pub slack_bot_token: Option<String>,
    pub slack_webhook_url: Option<String>,

    pub telegram_bot_token: Option<Zeroizing<String>>,

    pub discord_bot_token: Option<String>,
    pub discord_webhook_url: Option<String>,

    pub matrix_access_token: Option<String>,
    pub matrix_homeserver: Option<String>,
    pub matrix_user_id: Option<String>,

    pub mattermost_token: Option<String>,
    pub mattermost_server_url: Option<String>,

    pub whatsapp_access_token: Option<String>,
    pub whatsapp_phone_number_id: Option<String>,
    pub whatsapp_verify_token: Option<String>,

    // -- SMTP --
    pub smtp_host: Option<String>,
    pub smtp_user: Option<String>,
    pub smtp_pass: Option<Zeroizing<String>>,
    pub smtp_from: Option<String>,

    // -- Webhook outbound --
    pub outbound_webhook_url: Option<String>,

    // -- Observability --
    pub otel_endpoint: Option<String>,
    pub log_format_json: bool,

    // -- Runtime --
    pub drain_grace_secs: u64,
    pub canary_weight: f64,

    // -- Browser --
    pub chrome_path: Option<String>,

    // -- OCI registry --
    pub registry_auth: Option<String>,
}

impl PlatformEnv {
    /// Load configuration from `~/.rune/config.toml`.
    pub fn from_env() -> Result<Self, EnvError> {
        let config = load_config_file();
        Self::from_config(&config)
    }

    /// Build `PlatformEnv` from an explicit key-value map.
    pub fn from_config(config: &HashMap<String, String>) -> Result<Self, EnvError> {
        let env = Self {
            // Gateway
            gateway_base_url: val_or(config, "GATEWAY_BASE_URL", "http://localhost:3000").to_string(),
            agent_workspace_dir: val(config, "AGENT_WORKSPACE_DIR").map(str::to_string),
            agent_packages_dir: val(config, "AGENT_PACKAGES_DIR").map(str::to_string),

            // Auth
            api_key: val(config, "RUNE_API_KEY").map(|s| Zeroizing::new(s.to_string())),
            jwt_secret: val(config, "RUNE_JWT_SECRET").map(|s| Zeroizing::new(s.to_string())),

            // Rate limiting
            rate_limit_rps: parse_val(config, "RUNE_RATE_LIMIT_RPS", 100.0)?,
            rate_limit_burst: parse_val(config, "RUNE_RATE_LIMIT_BURST", 200)?,

            // LLM
            openai_api_key: val(config, "OPENAI_API_KEY").map(|s| Zeroizing::new(s.to_string())),
            openai_model: val_or(config, "OPENAI_MODEL", "gpt-4o-mini").to_string(),
            openai_base_url: val_or(config, "OPENAI_BASE_URL", "https://api.openai.com").to_string(),
            anthropic_api_key: val(config, "ANTHROPIC_API_KEY").map(|s| Zeroizing::new(s.to_string())),
            claude_code_api_key: val(config, "CLAUDE_API_KEY").map(|s| Zeroizing::new(s.to_string())),
            gemini_api_key: val(config, "GEMINI_API_KEY").map(|s| Zeroizing::new(s.to_string())),
            gemini_model: val_or(config, "GEMINI_MODEL", "gemini-2.0-flash").to_string(),
            copilot_api_key: val(config, "GITHUB_TOKEN")
                .or_else(|| val(config, "COPILOT_TOKEN"))
                .map(|s| Zeroizing::new(s.to_string())),

            // Storage
            object_store_url: val(config, "RUNE_OBJECT_STORE_URL").map(str::to_string),
            checkpoint_storage_threshold: parse_val(
                config,
                "RUNE_CHECKPOINT_STORAGE_THRESHOLD",
                1_048_576,
            )?,

            // Channels
            channels_config_path: val(config, "RUNE_CHANNELS_CONFIG").map(str::to_string),
            channel_agent: val(config, "RUNE_CHANNEL_AGENT").map(str::to_string),
            webhook_secret: val(config, "RUNE_WEBHOOK_SECRET").map(str::to_string),
            webhook_port: val(config, "RUNE_WEBHOOK_PORT").map(str::to_string),
            webhook_callback_url: val(config, "RUNE_WEBHOOK_CALLBACK_URL").map(str::to_string),

            slack_app_token: val(config, "RUNE_SLACK_APP_TOKEN").map(str::to_string),
            slack_bot_token: val(config, "RUNE_SLACK_BOT_TOKEN").map(str::to_string),
            slack_webhook_url: val(config, "RUNE_SLACK_WEBHOOK_URL").map(str::to_string),

            telegram_bot_token: val(config, "RUNE_TELEGRAM_BOT_TOKEN").map(|s| Zeroizing::new(s.to_string())),

            discord_bot_token: val(config, "RUNE_DISCORD_BOT_TOKEN").map(str::to_string),
            discord_webhook_url: val(config, "RUNE_DISCORD_WEBHOOK_URL").map(str::to_string),

            matrix_access_token: val(config, "RUNE_MATRIX_ACCESS_TOKEN").map(str::to_string),
            matrix_homeserver: val(config, "RUNE_MATRIX_HOMESERVER").map(str::to_string),
            matrix_user_id: val(config, "RUNE_MATRIX_USER_ID").map(str::to_string),

            mattermost_token: val(config, "RUNE_MATTERMOST_TOKEN").map(str::to_string),
            mattermost_server_url: val(config, "RUNE_MATTERMOST_SERVER_URL").map(str::to_string),

            whatsapp_access_token: val(config, "RUNE_WHATSAPP_ACCESS_TOKEN").map(str::to_string),
            whatsapp_phone_number_id: val(config, "RUNE_WHATSAPP_PHONE_NUMBER_ID").map(str::to_string),
            whatsapp_verify_token: val(config, "RUNE_WHATSAPP_VERIFY_TOKEN").map(str::to_string),

            // SMTP
            smtp_host: val(config, "RUNE_SMTP_HOST").map(str::to_string),
            smtp_user: val(config, "RUNE_SMTP_USER").map(str::to_string),
            smtp_pass: val(config, "RUNE_SMTP_PASS").map(|s| Zeroizing::new(s.to_string())),
            smtp_from: val(config, "RUNE_SMTP_FROM").map(str::to_string),

            // Webhook outbound
            outbound_webhook_url: val(config, "RUNE_WEBHOOK_URL").map(str::to_string),

            // Observability
            otel_endpoint: val(config, "OTEl_EXPORTER_OTLP_ENDPOINT").map(str::to_string),
            log_format_json: val(config, "RUST_LOG_FORMAT") == Some("json"),

            // Runtime
            drain_grace_secs: parse_val(config, "RUNE_DRAIN_GRACE_SECS", 30)?,
            canary_weight: parse_val(config, "RUNE_CANARY_WEIGHT", 0.1)?,

            // Browser
            chrome_path: val(config, "CHROME_PATH").map(str::to_string),

            // OCI
            registry_auth: val(config, "REGISTRY_AUTH").map(str::to_string),
        };

        env.validate()?;
        Ok(env)
    }

    /// Check cross-field invariants.
    fn validate(&self) -> Result<(), EnvError> {
        if self.rate_limit_rps <= 0.0 {
            return Err(EnvError::Invalid {
                var: "RUNE_RATE_LIMIT_RPS".into(),
                reason: "must be positive".into(),
            });
        }
        if self.canary_weight < 0.0 || self.canary_weight > 1.0 {
            return Err(EnvError::Invalid {
                var: "RUNE_CANARY_WEIGHT".into(),
                reason: "must be between 0.0 and 1.0".into(),
            });
        }
        Ok(())
    }

    pub fn auth_enabled(&self) -> bool {
        self.api_key.is_some() || self.jwt_secret.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn test_defaults() {
        let env = PlatformEnv::from_config(&HashMap::new()).expect("should succeed with defaults");
        assert_eq!(env.gateway_base_url, "http://localhost:3000");
        assert_eq!(env.openai_model, "gpt-4o-mini");
        assert_eq!(env.openai_base_url, "https://api.openai.com");
        assert_eq!(env.rate_limit_rps, 100.0);
        assert_eq!(env.rate_limit_burst, 200);
        assert_eq!(env.drain_grace_secs, 30);
        assert_eq!(env.canary_weight, 0.1);
        assert_eq!(env.checkpoint_storage_threshold, 1_048_576);
        assert!(!env.log_format_json);
        assert!(env.api_key.is_none());
        assert!(env.jwt_secret.is_none());
        assert!(env.openai_api_key.is_none());
        assert!(env.anthropic_api_key.is_none());
    }

    #[test]
    fn test_custom_values() {
        let config = cfg(&[
            ("GATEWAY_BASE_URL", "https://gw.example.com"),
            ("RUNE_API_KEY", "test-key-123"),
            ("OPENAI_API_KEY", "sk-test"),
            ("OPENAI_MODEL", "gpt-4"),
            ("RUNE_RATE_LIMIT_RPS", "50.5"),
            ("RUNE_RATE_LIMIT_BURST", "100"),
            ("RUST_LOG_FORMAT", "json"),
            ("RUNE_CANARY_WEIGHT", "0.25"),
        ]);

        let env = PlatformEnv::from_config(&config).expect("should succeed");
        assert_eq!(env.gateway_base_url, "https://gw.example.com");
        assert_eq!(env.api_key.as_ref().unwrap().as_str(), "test-key-123");
        assert_eq!(env.openai_api_key.as_ref().unwrap().as_str(), "sk-test");
        assert_eq!(env.openai_model, "gpt-4");
        assert_eq!(env.rate_limit_rps, 50.5);
        assert_eq!(env.rate_limit_burst, 100);
        assert!(env.log_format_json);
        assert_eq!(env.canary_weight, 0.25);
        assert!(env.auth_enabled());
    }

    #[test]
    fn test_validation_invalid_rps() {
        let config = cfg(&[("RUNE_RATE_LIMIT_RPS", "-5")]);
        let result = PlatformEnv::from_config(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("RUNE_RATE_LIMIT_RPS"));
    }

    #[test]
    fn test_validation_invalid_canary_weight() {
        let config = cfg(&[("RUNE_CANARY_WEIGHT", "1.5")]);
        let result = PlatformEnv::from_config(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("RUNE_CANARY_WEIGHT"));
    }

    #[test]
    fn test_parse_error() {
        let config = cfg(&[("RUNE_RATE_LIMIT_BURST", "not_a_number")]);
        let result = PlatformEnv::from_config(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("RUNE_RATE_LIMIT_BURST"));
    }

    #[test]
    fn test_empty_values_treated_as_none() {
        let env = PlatformEnv::from_config(&HashMap::new()).expect("should succeed");
        assert!(env.api_key.is_none());
        assert!(env.openai_api_key.is_none());
        assert!(!env.auth_enabled());
    }
}
