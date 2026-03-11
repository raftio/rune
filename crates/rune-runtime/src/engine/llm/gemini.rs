//! Google Gemini provider with streaming support.
//!
//! Auth: `GEMINI_API_KEY` (passed as query param).
//! Endpoint: `https://generativelanguage.googleapis.com/v1beta/models/{model}`.

use std::collections::HashMap;

use async_trait::async_trait;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio_util::io::StreamReader;
use futures::TryStreamExt;

use crate::error::RuntimeError;
use super::{ApiTool, ContentBlock, LlmProvider, LlmResponse, StreamChunk};

const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";

pub struct GeminiClient {
    api_key: String,
    pub model: String,
    http: reqwest::Client,
}

impl GeminiClient {
    pub fn new(api_key: String, model: String) -> Self {
        Self { api_key, model, http: reqwest::Client::new() }
    }

    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("GEMINI_API_KEY").ok()?;
        let model = std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-2.0-flash".into());
        Some(Self::new(api_key, model))
    }

    pub fn from_platform_env(env: &rune_env::PlatformEnv) -> Option<Self> {
        env.gemini_api_key
            .as_ref()
            .map(|k| Self::new(k.as_str().to_string(), env.gemini_model.clone()))
    }

    fn endpoint(&self, stream: bool) -> String {
        let method = if stream { "streamGenerateContent" } else { "generateContent" };
        format!("{}/{}:{}?key={}", GEMINI_BASE_URL, self.model, method, self.api_key)
    }

    /// Convert internal Anthropic-style messages to Gemini `contents` format.
    fn to_gemini_messages(messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for msg in messages {
            let role = msg["role"].as_str().unwrap_or("user");
            let content = &msg["content"];
            match role {
                "user" => {
                    let text = if let Some(t) = content.as_str() {
                        t.to_string()
                    } else if let Some(arr) = content.as_array() {
                        arr.iter()
                            .filter_map(|b| if b["type"] == "text" { b["text"].as_str().map(str::to_string) } else { None })
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else {
                        content.to_string()
                    };
                    out.push(serde_json::json!({
                        "role": "user",
                        "parts": [{"text": text}]
                    }));
                }
                "assistant" => {
                    if let Some(blocks) = content.as_array() {
                        let mut parts = Vec::new();
                        for b in blocks {
                            if b["type"] == "text" {
                                if let Some(text) = b["text"].as_str() {
                                    parts.push(serde_json::json!({"text": text}));
                                }
                            } else if b["type"] == "tool_use" {
                                parts.push(serde_json::json!({
                                    "functionCall": {
                                        "name": b["name"],
                                        "args": b["input"]
                                    }
                                }));
                            }
                        }
                        if !parts.is_empty() {
                            out.push(serde_json::json!({
                                "role": "model",
                                "parts": parts
                            }));
                        }
                    }
                }
                "tool" => {
                    if let Some(blocks) = content.as_array() {
                        let mut parts = Vec::new();
                        for b in blocks {
                            if b["type"] == "tool_result" {
                                let result = b["content"].as_str()
                                    .map(|s| serde_json::json!({"text": s}))
                                    .unwrap_or_else(|| b["content"].clone());
                                parts.push(serde_json::json!({
                                    "functionResponse": {
                                        "name": b["tool_use_id"],
                                        "response": {"content": result}
                                    }
                                }));
                            }
                        }
                        if !parts.is_empty() {
                            out.push(serde_json::json!({
                                "role": "user",
                                "parts": parts
                            }));
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn gemini_tools(tools: &[ApiTool]) -> serde_json::Value {
        if tools.is_empty() {
            return serde_json::json!([]);
        }
        let decls: Vec<serde_json::Value> = tools.iter().map(|t| serde_json::json!({
            "name": t.name,
            "description": t.description,
            "parameters": t.input_schema,
        })).collect();
        serde_json::json!([{"functionDeclarations": decls}])
    }

    fn build_body(
        system: &str,
        messages: &[serde_json::Value],
        tools: &[ApiTool],
        max_tokens: u32,
    ) -> serde_json::Value {
        let mut body = serde_json::json!({
            "contents": Self::to_gemini_messages(messages),
            "generationConfig": { "maxOutputTokens": max_tokens },
        });
        if !system.is_empty() {
            body["systemInstruction"] = serde_json::json!({
                "parts": [{"text": system}]
            });
        }
        let tool_list = Self::gemini_tools(tools);
        if !tools.is_empty() {
            body["tools"] = tool_list;
        }
        body
    }

    fn parse_finish_reason(candidate: &serde_json::Value, has_tool_use: bool) -> String {
        if has_tool_use {
            return "tool_use".into();
        }
        match candidate["finishReason"].as_str().unwrap_or("STOP") {
            "MAX_TOKENS" => "max_tokens",
            _ => "end_turn",
        }.into()
    }
}

#[async_trait]
impl LlmProvider for GeminiClient {
    fn default_model(&self) -> &str {
        &self.model
    }

    fn provider_name(&self) -> &str {
        "gemini"
    }

    async fn call(
        &self,
        _model: &str,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[ApiTool],
        max_tokens: u32,
    ) -> Result<LlmResponse, RuntimeError> {
        let body = Self::build_body(system, messages, tools, max_tokens);
        let resp = self.http.post(self.endpoint(false))
            .json(&body)
            .send()
            .await
            .map_err(|e| RuntimeError::Engine(format!("Gemini HTTP: {e}")))?;

        if !resp.status().is_success() {
            let s = resp.status();
            let b = resp.text().await.unwrap_or_default();
            return Err(RuntimeError::Engine(format!("Gemini {s}: {b}")));
        }

        let val: serde_json::Value = resp.json().await
            .map_err(|e| RuntimeError::Engine(format!("Gemini parse: {e}")))?;

        let candidate = val["candidates"]
            .get(0)
            .ok_or_else(|| RuntimeError::Engine("Gemini: no candidates in response".into()))?;

        let mut content = Vec::new();
        if let Some(parts) = candidate["content"]["parts"].as_array() {
            for part in parts {
                if let Some(text) = part["text"].as_str() {
                    if !text.is_empty() {
                        content.push(ContentBlock::Text { text: text.to_string() });
                    }
                } else if !part["functionCall"].is_null() {
                    let fc = &part["functionCall"];
                    let name = fc["name"].as_str().unwrap_or("").to_string();
                    content.push(ContentBlock::ToolUse {
                        id: format!("gemini_{name}"),
                        name,
                        input: fc["args"].clone(),
                    });
                }
            }
        }

        let has_tool_use = content.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. }));
        let stop_reason = Self::parse_finish_reason(candidate, has_tool_use);

        Ok(LlmResponse { stop_reason, content })
    }

    async fn stream(
        &self,
        _model: &str,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[ApiTool],
        max_tokens: u32,
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<(), RuntimeError> {
        let body = Self::build_body(system, messages, tools, max_tokens);

        // Gemini streaming requires `alt=sse` on the streamGenerateContent endpoint.
        let url = format!("{}&alt=sse", self.endpoint(true));
        let resp = self.http.post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                let err = RuntimeError::Engine(format!("Gemini HTTP: {e}"));
                on_chunk(StreamChunk::Error(err.to_string()));
                err
            })?;

        if !resp.status().is_success() {
            let s = resp.status();
            let b = resp.text().await.unwrap_or_default();
            let err = RuntimeError::Engine(format!("Gemini {s}: {b}"));
            on_chunk(StreamChunk::Error(err.to_string()));
            return Err(err);
        }

        let byte_stream = resp.bytes_stream()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
        let reader = StreamReader::new(byte_stream);
        let mut lines = BufReader::new(reader).lines();

        let mut full_text = String::new();
        // tool name → args accumulator
        let mut tool_bufs: HashMap<String, serde_json::Value> = HashMap::new();
        let mut stop_reason = "end_turn".to_string();

        while let Some(line) = lines.next_line().await.map_err(RuntimeError::Io)? {
            let data = match line.strip_prefix("data: ") { Some(d) => d.trim(), None => continue };
            if data.is_empty() { continue; }
            let val: serde_json::Value = match serde_json::from_str(data) { Ok(v) => v, Err(_) => continue };

            if let Some(parts) = val["candidates"]
                .get(0)
                .and_then(|c| c["content"]["parts"].as_array())
            {
                for part in parts {
                    if let Some(text) = part["text"].as_str() {
                        if !text.is_empty() {
                            full_text.push_str(text);
                            on_chunk(StreamChunk::Token(text.to_string()));
                        }
                    } else if !part["functionCall"].is_null() {
                        let fc = &part["functionCall"];
                        let name = fc["name"].as_str().unwrap_or("").to_string();
                        tool_bufs.insert(name, fc["args"].clone());
                        stop_reason = "tool_use".to_string();
                    }
                }
            }
        }

        for (name, input) in tool_bufs {
            let id = format!("gemini_{name}");
            on_chunk(StreamChunk::ToolUse { id, name, input });
        }

        on_chunk(StreamChunk::Done { stop_reason, full_text });
        Ok(())
    }
}
