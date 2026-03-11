//! Claude Code provider — uses the Anthropic API with a Claude Code access token.
//!
//! Auth: `CLAUDE_API_KEY` (Claude Code OAuth access token).

use async_trait::async_trait;

use crate::error::RuntimeError;
use super::{ApiTool, LlmProvider, LlmResponse, StreamChunk};
use super::anthropic::AnthropicClient;

pub struct ClaudeCodeClient(AnthropicClient);

impl ClaudeCodeClient {
    pub fn new(api_key: String) -> Self {
        Self(AnthropicClient::new(api_key))
    }

    pub fn from_env() -> Option<Self> {
        std::env::var("CLAUDE_API_KEY").ok().map(Self::new)
    }

    pub fn from_platform_env(env: &rune_env::PlatformEnv) -> Option<Self> {
        env.claude_code_api_key
            .as_ref()
            .map(|k| Self::new(k.as_str().to_string()))
    }
}

#[async_trait]
impl LlmProvider for ClaudeCodeClient {
    fn default_model(&self) -> &str {
        "claude-sonnet-4-6"
    }

    fn provider_name(&self) -> &str {
        "claude-code"
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
