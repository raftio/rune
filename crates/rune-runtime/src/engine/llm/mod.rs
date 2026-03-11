//! LLM provider abstraction and common types.

mod anthropic;
mod claude_code;
mod copilot;
mod gemini;
mod openai;

pub use anthropic::AnthropicClient;
pub use claude_code::ClaudeCodeClient;
pub use copilot::CopilotClient;
pub use gemini::GeminiClient;
pub use openai::OpenAiClient;

use async_trait::async_trait;
use rune_spec::ToolDescriptor;
use serde::{Deserialize, Serialize};

use crate::error::RuntimeError;

// ---------------------------------------------------------------------------
// Common response types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

pub struct LlmResponse {
    pub stop_reason: String,
    pub content: Vec<ContentBlock>,
}

/// A chunk produced during streaming.
#[derive(Debug)]
pub enum StreamChunk {
    Token(String),
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    Done {
        stop_reason: String,
        full_text: String,
    },
    Error(String),
}

// ---------------------------------------------------------------------------
// Tool schema
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ApiTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

pub fn tools_to_api(descriptors: &[ToolDescriptor]) -> Vec<ApiTool> {
    descriptors
        .iter()
        .map(|t| {
            if t.is_builtin() {
                if let Some(def) = rune_tools::find_definition(&t.name) {
                    ApiTool {
                        name: rune_tools::to_wire_name(&t.name),
                        description: def.description.to_string(),
                        input_schema: def.input_schema,
                    }
                } else {
                    ApiTool {
                        name: rune_tools::to_wire_name(&t.name),
                        description: format!("{} (builtin)", t.name),
                        input_schema: serde_json::json!({
                            "type": "object", "properties": {}, "additionalProperties": true
                        }),
                    }
                }
            } else {
                ApiTool {
                    name: t.name.clone(),
                    description: format!("{} (v{})", t.name, t.version),
                    input_schema: serde_json::json!({
                        "type": "object", "properties": {}, "additionalProperties": true
                    }),
                }
            }
        })
        .collect()
}

pub fn user_input_to_content(input: &serde_json::Value) -> serde_json::Value {
    if let Some(text) = input.as_str() {
        serde_json::json!([{"type":"text","text":text}])
    } else if input.is_array() {
        input.clone()
    } else {
        serde_json::json!([{"type":"text","text":input.to_string()}])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune_spec::{ToolDescriptor, ToolRuntime};

    // --- user_input_to_content ---

    #[test]
    fn string_input_becomes_text_block_array() {
        let out = user_input_to_content(&serde_json::json!("hello world"));
        assert!(out.is_array());
        let arr = out.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "hello world");
    }

    #[test]
    fn array_input_passes_through_unchanged() {
        let input = serde_json::json!([{"type": "text", "text": "already array"}]);
        let out = user_input_to_content(&input);
        assert_eq!(out, input);
    }

    #[test]
    fn object_input_serialized_as_text_block() {
        let input = serde_json::json!({"key": "value"});
        let out = user_input_to_content(&input);
        assert!(out.is_array());
        let arr = out.as_array().unwrap();
        assert_eq!(arr[0]["type"], "text");
        let text = arr[0]["text"].as_str().unwrap();
        assert!(text.contains("key"));
        assert!(text.contains("value"));
    }

    #[test]
    fn empty_string_becomes_empty_text_block() {
        let out = user_input_to_content(&serde_json::json!(""));
        let arr = out.as_array().unwrap();
        assert_eq!(arr[0]["text"], "");
    }

    // --- ContentBlock serialization ---

    #[test]
    fn content_block_text_serializes_with_type_field() {
        let block = ContentBlock::Text {
            text: "hello".into(),
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "text");
        assert_eq!(json["text"], "hello");
    }

    #[test]
    fn content_block_tool_use_serializes_correctly() {
        let block = ContentBlock::ToolUse {
            id: "call_123".into(),
            name: "rune__shell".into(),
            input: serde_json::json!({"cmd": "ls"}),
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "tool_use");
        assert_eq!(json["id"], "call_123");
        assert_eq!(json["name"], "rune__shell");
        assert_eq!(json["input"]["cmd"], "ls");
    }

    #[test]
    fn content_block_text_deserializes() {
        let json = serde_json::json!({"type": "text", "text": "reply"});
        let block: ContentBlock = serde_json::from_value(json).unwrap();
        assert!(matches!(block, ContentBlock::Text { text } if text == "reply"));
    }

    #[test]
    fn content_block_tool_use_deserializes() {
        let json = serde_json::json!({
            "type": "tool_use",
            "id": "id1",
            "name": "my_tool",
            "input": {"x": 1}
        });
        let block: ContentBlock = serde_json::from_value(json).unwrap();
        assert!(matches!(block, ContentBlock::ToolUse { ref name, .. } if name == "my_tool"));
    }

    // --- tools_to_api ---

    #[test]
    fn tools_to_api_empty_list_returns_empty() {
        let result = tools_to_api(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn tools_to_api_custom_tool_uses_name_and_version() {
        let tool = ToolDescriptor {
            name: "my_search".into(),
            version: "1.2.3".into(),
            runtime: ToolRuntime::Process,
            module: "tools/search.py".into(),
            timeout_ms: 5_000,
            retry_policy: rune_spec::tool::RetryPolicy::default(),
            capabilities: vec![],
            input_schema_ref: None,
            output_schema_ref: None,
            agent_ref: None,
            max_depth: None,
            mcp_server: None,
        };
        let api_tools = tools_to_api(&[tool]);
        assert_eq!(api_tools.len(), 1);
        assert_eq!(api_tools[0].name, "my_search");
        assert!(api_tools[0].description.contains("my_search"));
        assert!(api_tools[0].description.contains("1.2.3"));
    }

    #[test]
    fn tools_to_api_builtin_tool_uses_wire_name() {
        let tool = ToolDescriptor::builtin("rune@shell");
        let api_tools = tools_to_api(&[tool]);
        assert_eq!(api_tools.len(), 1);
        // wire name converts rune@ prefix to rune__
        let wire = rune_tools::to_wire_name("rune@shell");
        assert_eq!(api_tools[0].name, wire);
    }

    #[test]
    fn tools_to_api_custom_tool_schema_is_open_object() {
        let tool = ToolDescriptor {
            name: "custom".into(),
            version: "0.1.0".into(),
            runtime: ToolRuntime::Process,
            module: String::new(),
            timeout_ms: 5_000,
            retry_policy: rune_spec::tool::RetryPolicy::default(),
            capabilities: vec![],
            input_schema_ref: None,
            output_schema_ref: None,
            agent_ref: None,
            max_depth: None,
            mcp_server: None,
        };
        let api_tools = tools_to_api(&[tool]);
        assert_eq!(api_tools[0].input_schema["type"], "object");
        assert_eq!(api_tools[0].input_schema["additionalProperties"], true);
    }
}

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn default_model(&self) -> &str;
    fn provider_name(&self) -> &str;

    async fn call(
        &self,
        model: &str,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[ApiTool],
        max_tokens: u32,
    ) -> Result<LlmResponse, RuntimeError>;

    async fn stream(
        &self,
        model: &str,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[ApiTool],
        max_tokens: u32,
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<(), RuntimeError>;
}

// ---------------------------------------------------------------------------
// Unified LlmClient
// ---------------------------------------------------------------------------

pub struct LlmClient {
    inner: Box<dyn LlmProvider>,
}

impl LlmClient {
    pub fn from_env() -> Option<Self> {
        if let Some(c) = AnthropicClient::from_env() {
            return Some(Self { inner: Box::new(c) });
        }
        if let Some(c) = ClaudeCodeClient::from_env() {
            return Some(Self { inner: Box::new(c) });
        }
        if let Some(c) = GeminiClient::from_env() {
            return Some(Self { inner: Box::new(c) });
        }
        if let Some(c) = CopilotClient::from_env() {
            return Some(Self { inner: Box::new(c) });
        }
        if let Some(c) = OpenAiClient::from_env() {
            return Some(Self { inner: Box::new(c) });
        }
        None
    }

    pub fn from_platform_env(env: &rune_env::PlatformEnv) -> Option<Self> {
        if let Some(key) = &env.anthropic_api_key {
            return Some(Self {
                inner: Box::new(AnthropicClient::new(key.as_str().to_string())),
            });
        }
        if let Some(c) = ClaudeCodeClient::from_platform_env(env) {
            return Some(Self { inner: Box::new(c) });
        }
        if let Some(c) = GeminiClient::from_platform_env(env) {
            return Some(Self { inner: Box::new(c) });
        }
        if let Some(c) = CopilotClient::from_platform_env(env) {
            return Some(Self { inner: Box::new(c) });
        }
        if let Some(key) = &env.openai_api_key {
            return Some(Self {
                inner: Box::new(OpenAiClient::new(
                    key.as_str().to_string(),
                    env.openai_model.clone(),
                )),
            });
        }
        None
    }

    pub fn default_model(&self) -> &str {
        self.inner.default_model()
    }

    pub fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }

    pub async fn call(
        &self,
        model_override: Option<&str>,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[ApiTool],
        max_tokens: u32,
    ) -> Result<LlmResponse, RuntimeError> {
        let model = model_override.unwrap_or_else(|| self.inner.default_model());
        let provider = self.inner.provider_name();
        let start = std::time::Instant::now();
        let result = self
            .inner
            .call(model, system, messages, tools, max_tokens)
            .await;
        crate::metrics::record_model_call_duration(provider, model, start.elapsed().as_secs_f64());
        result
    }

    pub async fn stream(
        &self,
        model_override: Option<&str>,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[ApiTool],
        max_tokens: u32,
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<(), RuntimeError> {
        let model = model_override.unwrap_or_else(|| self.inner.default_model());
        self.inner
            .stream(model, system, messages, tools, max_tokens, on_chunk)
            .await
    }
}
