use std::collections::HashMap;

use crate::platform::PlatformEnv;

/// Resolved environment variables for an agent container/process.
///
/// Merges platform-injected vars with user-defined vars from the
/// deployment's `env_json`. User-defined values take precedence.
#[derive(Debug, Clone)]
pub struct AgentEnv {
    vars: HashMap<String, String>,
}

impl AgentEnv {
    /// Build the merged env map for an agent deployment.
    ///
    /// 1. Start with platform vars agents commonly need (e.g. `GATEWAY_BASE_URL`).
    /// 2. Layer user-defined env from the deployment's `env_json` column.
    /// User values win on conflict.
    pub fn resolve(platform: &PlatformEnv, env_json: Option<&str>) -> Self {
        let mut vars = HashMap::new();

        // Platform vars that agent processes/containers typically need
        vars.insert(
            "GATEWAY_BASE_URL".into(),
            platform.gateway_base_url.clone(),
        );
        if let Some(dir) = &platform.agent_workspace_dir {
            vars.insert("AGENT_WORKSPACE_DIR".into(), dir.clone());
        }
        if let Some(dir) = &platform.agent_packages_dir {
            vars.insert("AGENT_PACKAGES_DIR".into(), dir.clone());
        }

        // User-defined env from deployment (wins on conflict)
        if let Some(json) = env_json {
            if let Ok(user_vars) = serde_json::from_str::<HashMap<String, String>>(json) {
                vars.extend(user_vars);
            }
        }

        Self { vars }
    }

    pub fn to_map(&self) -> HashMap<String, String> {
        self.vars.clone()
    }

    /// Format suitable for `Vec<(String, String)>`, e.g. `Command::envs()`.
    pub fn to_vec(&self) -> Vec<(String, String)> {
        self.vars.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// Format suitable for Docker/bollard: `Vec<String>` of `"KEY=VALUE"`.
    pub fn to_docker_env(&self) -> Vec<String> {
        self.vars.iter().map(|(k, v)| format!("{k}={v}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_platform_env() -> PlatformEnv {
        PlatformEnv {
            gateway_base_url: "http://test:3000".into(),
            agent_workspace_dir: Some("/workspace".into()),
            agent_packages_dir: Some("/packages".into()),
            api_key: None,
            jwt_secret: None,
            rate_limit_rps: 100.0,
            rate_limit_burst: 200,
            openai_api_key: None,
            openai_model: "gpt-4o-mini".into(),
            openai_base_url: "https://api.openai.com".into(),
            anthropic_api_key: None,
            claude_code_api_key: None,
            gemini_api_key: None,
            gemini_model: "gemini-2.0-flash".into(),
            copilot_api_key: None,
            object_store_url: None,
            checkpoint_storage_threshold: 1_048_576,
            channels_config_path: None,
            channel_agent: None,
            webhook_secret: None,
            webhook_port: None,
            webhook_callback_url: None,
            slack_app_token: None,
            slack_bot_token: None,
            slack_webhook_url: None,
            telegram_bot_token: None,
            discord_bot_token: None,
            discord_webhook_url: None,
            matrix_access_token: None,
            matrix_homeserver: None,
            matrix_user_id: None,
            mattermost_token: None,
            mattermost_server_url: None,
            whatsapp_access_token: None,
            whatsapp_phone_number_id: None,
            whatsapp_verify_token: None,
            smtp_host: None,
            smtp_user: None,
            smtp_pass: None,
            smtp_from: None,
            outbound_webhook_url: None,
            otel_endpoint: None,
            log_format_json: false,
            drain_grace_secs: 30,
            canary_weight: 0.1,
            chrome_path: None,
            registry_auth: None,
        }
    }

    #[test]
    fn test_resolve_injects_platform_vars() {
        let platform = test_platform_env();
        let agent = AgentEnv::resolve(&platform, None);
        let map = agent.to_map();

        assert_eq!(map.get("GATEWAY_BASE_URL").unwrap(), "http://test:3000");
        assert_eq!(map.get("AGENT_WORKSPACE_DIR").unwrap(), "/workspace");
        assert_eq!(map.get("AGENT_PACKAGES_DIR").unwrap(), "/packages");
    }

    #[test]
    fn test_resolve_user_env_wins() {
        let platform = test_platform_env();
        let user_env = r#"{"GATEWAY_BASE_URL": "http://custom:9999", "MY_VAR": "hello"}"#;
        let agent = AgentEnv::resolve(&platform, Some(user_env));
        let map = agent.to_map();

        // User value wins
        assert_eq!(map.get("GATEWAY_BASE_URL").unwrap(), "http://custom:9999");
        assert_eq!(map.get("MY_VAR").unwrap(), "hello");
        // Platform vars still present
        assert_eq!(map.get("AGENT_WORKSPACE_DIR").unwrap(), "/workspace");
    }

    #[test]
    fn test_resolve_invalid_json_ignored() {
        let platform = test_platform_env();
        let agent = AgentEnv::resolve(&platform, Some("not valid json"));
        let map = agent.to_map();

        // Should still have platform vars
        assert_eq!(map.get("GATEWAY_BASE_URL").unwrap(), "http://test:3000");
    }

    #[test]
    fn test_to_docker_env() {
        let platform = test_platform_env();
        let agent = AgentEnv::resolve(&platform, Some(r#"{"FOO": "bar"}"#));
        let docker_env = agent.to_docker_env();

        assert!(docker_env.iter().any(|e| e == "FOO=bar"));
        assert!(docker_env.iter().any(|e| e.starts_with("GATEWAY_BASE_URL=")));
    }

    #[test]
    fn test_to_vec() {
        let platform = test_platform_env();
        let agent = AgentEnv::resolve(&platform, None);
        let vec = agent.to_vec();

        assert!(vec.iter().any(|(k, v)| k == "GATEWAY_BASE_URL" && v == "http://test:3000"));
    }

    #[test]
    fn test_resolve_no_workspace_dir() {
        let mut platform = test_platform_env();
        platform.agent_workspace_dir = None;
        platform.agent_packages_dir = None;

        let agent = AgentEnv::resolve(&platform, None);
        let map = agent.to_map();

        assert!(map.get("AGENT_WORKSPACE_DIR").is_none());
        assert!(map.get("AGENT_PACKAGES_DIR").is_none());
        assert_eq!(map.get("GATEWAY_BASE_URL").unwrap(), "http://test:3000");
    }
}