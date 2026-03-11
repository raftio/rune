//! Anthropic (Claude) provider with streaming support.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio_util::io::StreamReader;
use futures::TryStreamExt;

use crate::error::RuntimeError;
use super::{ApiTool, ContentBlock, LlmProvider, LlmResponse, StreamChunk};

pub struct AnthropicClient {
    api_key: String,
    http: reqwest::Client,
}

impl AnthropicClient {
    pub fn new(api_key: String) -> Self {
        Self { api_key, http: reqwest::Client::new() }
    }

    pub fn from_env() -> Option<Self> {
        std::env::var("ANTHROPIC_API_KEY").ok().map(Self::new)
    }

    pub fn from_platform_env(env: &rune_env::PlatformEnv) -> Option<Self> {
        env.anthropic_api_key.as_ref().map(|k| Self::new(k.as_str().to_string()))
    }

    fn endpoint() -> &'static str {
        "https://api.anthropic.com/v1/messages"
    }

    fn build_body(
        model: &str,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[ApiTool],
        max_tokens: u32,
        stream: bool,
    ) -> Result<serde_json::Value, RuntimeError> {
        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "system": system,
            "messages": messages,
        });
        if stream {
            body["stream"] = serde_json::json!(true);
        }
        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(tools)
                .map_err(|e| RuntimeError::Engine(e.to_string()))?;
        }
        Ok(body)
    }

    async fn send_request(&self, body: &serde_json::Value) -> Result<reqwest::Response, RuntimeError> {
        let resp = self.http.post(Self::endpoint())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(body)
            .send()
            .await
            .map_err(|e| RuntimeError::Engine(format!("Anthropic HTTP: {e}")))?;

        if !resp.status().is_success() {
            let s = resp.status();
            let b = resp.text().await.unwrap_or_default();
            return Err(RuntimeError::Engine(format!("Anthropic {s}: {b}")));
        }
        Ok(resp)
    }
}

#[async_trait]
impl LlmProvider for AnthropicClient {
    fn default_model(&self) -> &str {
        "claude-sonnet-4-6"
    }

    fn provider_name(&self) -> &str {
        "anthropic"
    }

    async fn call(
        &self,
        model: &str,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[ApiTool],
        max_tokens: u32,
    ) -> Result<LlmResponse, RuntimeError> {
        let body = Self::build_body(model, system, messages, tools, max_tokens, false)?;
        let resp = self.send_request(&body).await?;

        #[derive(Deserialize)]
        struct Raw { stop_reason: String, content: Vec<ContentBlock> }

        let raw: Raw = resp.json().await
            .map_err(|e| RuntimeError::Engine(format!("Anthropic parse: {e}")))?;
        Ok(LlmResponse { stop_reason: raw.stop_reason, content: raw.content })
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
        let body = Self::build_body(model, system, messages, tools, max_tokens, true)?;
        let resp = match self.send_request(&body).await {
            Ok(r) => r,
            Err(e) => {
                on_chunk(StreamChunk::Error(e.to_string()));
                return Err(e);
            }
        };

        let byte_stream = resp.bytes_stream()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
        let reader = StreamReader::new(byte_stream);
        let mut lines = BufReader::new(reader).lines();

        let mut tool_bufs: HashMap<u64, (String, String, String)> = HashMap::new();
        let mut full_text = String::new();
        let mut stop_reason = "end_turn".to_string();

        while let Some(line) = lines.next_line().await.map_err(RuntimeError::Io)? {
            let data = match line.strip_prefix("data: ") { Some(d) => d.trim(), None => continue };
            if data.is_empty() { continue; }
            let val: serde_json::Value = match serde_json::from_str(data) { Ok(v) => v, Err(_) => continue };

            match val["type"].as_str() {
                Some("content_block_start") => {
                    let idx = val["index"].as_u64().unwrap_or(0);
                    if val["content_block"]["type"] == "tool_use" {
                        let id = val["content_block"]["id"].as_str().unwrap_or("").to_string();
                        let name = val["content_block"]["name"].as_str().unwrap_or("").to_string();
                        tool_bufs.insert(idx, (id, name, String::new()));
                    }
                }
                Some("content_block_delta") => {
                    let idx = val["index"].as_u64().unwrap_or(0);
                    match val["delta"]["type"].as_str() {
                        Some("text_delta") => {
                            let text = val["delta"]["text"].as_str().unwrap_or("").to_string();
                            if !text.is_empty() {
                                full_text.push_str(&text);
                                on_chunk(StreamChunk::Token(text));
                            }
                        }
                        Some("input_json_delta") => {
                            if let Some(buf) = tool_bufs.get_mut(&idx) {
                                buf.2.push_str(val["delta"]["partial_json"].as_str().unwrap_or(""));
                            }
                        }
                        _ => {}
                    }
                }
                Some("content_block_stop") => {
                    let idx = val["index"].as_u64().unwrap_or(0);
                    if let Some((id, name, json)) = tool_bufs.remove(&idx) {
                        let input = serde_json::from_str(&json).unwrap_or_default();
                        on_chunk(StreamChunk::ToolUse { id, name, input });
                    }
                }
                Some("message_delta") => {
                    if let Some(sr) = val["delta"]["stop_reason"].as_str() {
                        stop_reason = sr.to_string();
                    }
                }
                _ => {}
            }
        }

        on_chunk(StreamChunk::Done { stop_reason, full_text });
        Ok(())
    }
}
