use crate::error::EnvError;
use zeroize::Zeroizing;

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

fn var_or(name: &str, default: &str) -> String {
    var(name).unwrap_or_else(|| default.to_string())
}

fn parse_var<T: std::str::FromStr>(name: &str, default: T) -> Result<T, EnvError> {
    match var(name) {
        Some(v) => v.parse().map_err(|_| EnvError::Invalid {
            var: name.to_string(),
            reason: format!("cannot parse '{v}'"),
        }),
        None => Ok(default),
    }
}

/// Centralized platform-level configuration read from environment variables.
///
/// Initialized once at startup and shared via `Arc<PlatformEnv>` across all
/// crates. Every `std::env::var()` call in the codebase should be replaced
/// by reading from this struct.
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
    /// Read all platform configuration from environment variables.
    pub fn from_env() -> Result<Self, EnvError> {
        let env = Self {
            // Gateway
            gateway_base_url: var_or("GATEWAY_BASE_URL", "http://localhost:3000"),
            agent_workspace_dir: var("AGENT_WORKSPACE_DIR"),
            agent_packages_dir: var("AGENT_PACKAGES_DIR"),

            // Auth
            api_key: var("RUNE_API_KEY").map(Zeroizing::new),
            jwt_secret: var("RUNE_JWT_SECRET").map(Zeroizing::new),

            // Rate limiting
            rate_limit_rps: parse_var("RUNE_RATE_LIMIT_RPS", 100.0)?,
            rate_limit_burst: parse_var("RUNE_RATE_LIMIT_BURST", 200)?,

            // LLM
            openai_api_key: var("OPENAI_API_KEY").map(Zeroizing::new),
            openai_model: var_or("OPENAI_MODEL", "gpt-4o-mini"),
            openai_base_url: var_or("OPENAI_BASE_URL", "https://api.openai.com"),
            anthropic_api_key: var("ANTHROPIC_API_KEY").map(Zeroizing::new),
            claude_code_api_key: var("CLAUDE_API_KEY").map(Zeroizing::new),
            gemini_api_key: var("GEMINI_API_KEY").map(Zeroizing::new),
            gemini_model: var_or("GEMINI_MODEL", "gemini-2.0-flash"),
            copilot_api_key: var("GITHUB_TOKEN")
                .or_else(|| var("COPILOT_TOKEN"))
                .map(Zeroizing::new),

            // Storage
            object_store_url: var("RUNE_OBJECT_STORE_URL"),
            checkpoint_storage_threshold: parse_var(
                "RUNE_CHECKPOINT_STORAGE_THRESHOLD",
                1_048_576,
            )?,

            // Channels
            channels_config_path: var("RUNE_CHANNELS_CONFIG"),
            channel_agent: var("RUNE_CHANNEL_AGENT"),
            webhook_secret: var("RUNE_WEBHOOK_SECRET"),
            webhook_port: var("RUNE_WEBHOOK_PORT"),
            webhook_callback_url: var("RUNE_WEBHOOK_CALLBACK_URL"),

            slack_app_token: var("RUNE_SLACK_APP_TOKEN"),
            slack_bot_token: var("RUNE_SLACK_BOT_TOKEN"),
            slack_webhook_url: var("RUNE_SLACK_WEBHOOK_URL"),

            telegram_bot_token: var("RUNE_TELEGRAM_BOT_TOKEN").map(Zeroizing::new),

            discord_bot_token: var("RUNE_DISCORD_BOT_TOKEN"),
            discord_webhook_url: var("RUNE_DISCORD_WEBHOOK_URL"),

            matrix_access_token: var("RUNE_MATRIX_ACCESS_TOKEN"),
            matrix_homeserver: var("RUNE_MATRIX_HOMESERVER"),
            matrix_user_id: var("RUNE_MATRIX_USER_ID"),

            mattermost_token: var("RUNE_MATTERMOST_TOKEN"),
            mattermost_server_url: var("RUNE_MATTERMOST_SERVER_URL"),

            whatsapp_access_token: var("RUNE_WHATSAPP_ACCESS_TOKEN"),
            whatsapp_phone_number_id: var("RUNE_WHATSAPP_PHONE_NUMBER_ID"),
            whatsapp_verify_token: var("RUNE_WHATSAPP_VERIFY_TOKEN"),

            // SMTP
            smtp_host: var("RUNE_SMTP_HOST"),
            smtp_user: var("RUNE_SMTP_USER"),
            smtp_pass: var("RUNE_SMTP_PASS").map(Zeroizing::new),
            smtp_from: var("RUNE_SMTP_FROM"),

            // Webhook outbound
            outbound_webhook_url: var("RUNE_WEBHOOK_URL"),

            // Observability
            otel_endpoint: var("OTEL_EXPORTER_OTLP_ENDPOINT"),
            log_format_json: var("RUST_LOG_FORMAT").as_deref() == Some("json"),

            // Runtime
            drain_grace_secs: parse_var("RUNE_DRAIN_GRACE_SECS", 30)?,
            canary_weight: parse_var("RUNE_CANARY_WEIGHT", 0.1)?,

            // Browser
            chrome_path: var("CHROME_PATH"),

            // OCI
            registry_auth: var("REGISTRY_AUTH"),
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
    use std::sync::Mutex;

    // Tests that touch env vars must be serialized to avoid races.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn clear_rune_env_vars() {
        for (key, _) in std::env::vars() {
            if key.starts_with("RUNE_")
                || key.starts_with("OPENAI_")
                || key.starts_with("ANTHROPIC_")
                || key.starts_with("GATEWAY_")
                || key.starts_with("AGENT_")
                || key.starts_with("CHROME_")
                || key.starts_with("REGISTRY_")
                || key == "OTEL_EXPORTER_OTLP_ENDPOINT"
                || key == "RUST_LOG_FORMAT"
            {
                std::env::remove_var(&key);
            }
        }
    }

    #[test]
    fn test_from_env_defaults() {
        let _lock = ENV_MUTEX.lock().unwrap();
        clear_rune_env_vars();

        let env = PlatformEnv::from_env().expect("should succeed with defaults");
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
    fn test_from_env_custom_values() {
        let _lock = ENV_MUTEX.lock().unwrap();
        clear_rune_env_vars();

        std::env::set_var("GATEWAY_BASE_URL", "https://gw.example.com");
        std::env::set_var("RUNE_API_KEY", "test-key-123");
        std::env::set_var("OPENAI_API_KEY", "sk-test");
        std::env::set_var("OPENAI_MODEL", "gpt-4");
        std::env::set_var("RUNE_RATE_LIMIT_RPS", "50.5");
        std::env::set_var("RUNE_RATE_LIMIT_BURST", "100");
        std::env::set_var("RUST_LOG_FORMAT", "json");
        std::env::set_var("RUNE_CANARY_WEIGHT", "0.25");

        let env = PlatformEnv::from_env().expect("should succeed");
        assert_eq!(env.gateway_base_url, "https://gw.example.com");
        assert_eq!(env.api_key.as_ref().unwrap().as_str(), "test-key-123");
        assert_eq!(env.openai_api_key.as_ref().unwrap().as_str(), "sk-test");
        assert_eq!(env.openai_model, "gpt-4");
        assert_eq!(env.rate_limit_rps, 50.5);
        assert_eq!(env.rate_limit_burst, 100);
        assert!(env.log_format_json);
        assert_eq!(env.canary_weight, 0.25);
        assert!(env.auth_enabled());

        clear_rune_env_vars();
    }

    #[test]
    fn test_validation_invalid_rps() {
        let _lock = ENV_MUTEX.lock().unwrap();
        clear_rune_env_vars();

        std::env::set_var("RUNE_RATE_LIMIT_RPS", "-5");

        let result = PlatformEnv::from_env();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("RUNE_RATE_LIMIT_RPS"));

        clear_rune_env_vars();
    }

    #[test]
    fn test_validation_invalid_canary_weight() {
        let _lock = ENV_MUTEX.lock().unwrap();
        clear_rune_env_vars();

        std::env::set_var("RUNE_CANARY_WEIGHT", "1.5");

        let result = PlatformEnv::from_env();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("RUNE_CANARY_WEIGHT"));

        clear_rune_env_vars();
    }

    #[test]
    fn test_parse_error() {
        let _lock = ENV_MUTEX.lock().unwrap();
        clear_rune_env_vars();

        std::env::set_var("RUNE_RATE_LIMIT_BURST", "not_a_number");

        let result = PlatformEnv::from_env();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("RUNE_RATE_LIMIT_BURST"));

        clear_rune_env_vars();
    }

    #[test]
    fn test_empty_values_treated_as_none() {
        let _lock = ENV_MUTEX.lock().unwrap();
        clear_rune_env_vars();

        std::env::set_var("RUNE_API_KEY", "");
        std::env::set_var("OPENAI_API_KEY", "");

        let env = PlatformEnv::from_env().expect("should succeed");
        assert!(env.api_key.is_none());
        assert!(env.openai_api_key.is_none());
        assert!(!env.auth_enabled());

        clear_rune_env_vars();
    }
}
