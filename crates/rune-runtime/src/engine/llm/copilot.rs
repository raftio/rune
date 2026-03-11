//! GitHub Copilot provider — OpenAI-compatible API.
//!
//! Auth: `GITHUB_TOKEN` or `COPILOT_TOKEN`.
//! Endpoint: `https://api.githubcopilot.com`.

use async_trait::async_trait;

use crate::error::RuntimeError;
use super::{ApiTool, LlmProvider, LlmResponse, StreamChunk};
use super::openai::OpenAiClient;

const COPILOT_BASE_URL: &str = "https://api.githubcopilot.com";
const COPILOT_DEFAULT_MODEL: &str = "gpt-4o";

pub struct CopilotClient(OpenAiClient);

impl CopilotClient {
    pub fn new(token: String) -> Self {
        Self(OpenAiClient::new_with_base_url(
            token,
            COPILOT_DEFAULT_MODEL.into(),
            COPILOT_BASE_URL.into(),
        ))
    }

    pub fn from_env() -> Option<Self> {
        std::env::var("GITHUB_TOKEN")
            .ok()
            .or_else(|| std::env::var("COPILOT_TOKEN").ok())
            .map(Self::new)
    }

    pub fn from_platform_env(env: &rune_env::PlatformEnv) -> Option<Self> {
        env.copilot_api_key
            .as_ref()
            .map(|k| Self::new(k.as_str().to_string()))
    }
}

#[async_trait]
impl LlmProvider for CopilotClient {
    fn default_model(&self) -> &str {
        COPILOT_DEFAULT_MODEL
    }

    fn provider_name(&self) -> &str {
        "copilot"
    }

    async fn call(
        &self,
        model: &str,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[ApiTool],
        max_tokens: u32,
    ) -> Result<LlmResponse, RuntimeError> {
        self.0.call(model, system, messages, tools, max_tokens).await
    }

    async fn stream(
        &self,
        model: &str,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[ApiTool],
        max_tokens: u32,
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<(), RuntimeError> {
        self.0.stream(model, system, messages, tools, max_tokens, on_chunk).await
    }
}
