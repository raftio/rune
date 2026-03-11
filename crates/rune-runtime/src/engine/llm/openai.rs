//! OpenAI provider with streaming support.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio_util::io::StreamReader;
use futures::TryStreamExt;

use crate::error::RuntimeError;
use super::{ApiTool, ContentBlock, LlmProvider, LlmResponse, StreamChunk};

fn extract_text_from_blocks(content: &serde_json::Value) -> String {
    if let Some(t) = content.as_str() { return t.to_string(); }
    if let Some(arr) = content.as_array() {
        return arr.iter()
            .filter_map(|b| {
                if b["type"] == "text" { b["text"].as_str().map(str::to_string) } else { None }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    content.to_string()
}

pub struct OpenAiClient {
    api_key: String,
    pub model: String,
    base_url: String,
    http: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            api_key,
            model,
            base_url: "https://api.openai.com".into(),
            http: reqwest::Client::new(),
        }
    }

    pub fn new_with_base_url(api_key: String, model: String, base_url: String) -> Self {
        Self { api_key, model, base_url, http: reqwest::Client::new() }
    }

    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("OPENAI_API_KEY").ok()?;
        let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());
        Some(Self::new(api_key, model))
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'))
    }

    /// Convert Anthropic-style messages to OpenAI chat format.
    fn to_openai_messages(messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for msg in messages {
            let role = msg["role"].as_str().unwrap_or("user");
            let content = &msg["content"];
            match role {
                "user" => {
                    out.push(serde_json::json!({
                        "role": "user",
                        "content": extract_text_from_blocks(content),
                    }));
                }
                "assistant" => {
                    if let Some(blocks) = content.as_array() {
                        let tcs: Vec<_> = blocks.iter().filter_map(|b| {
                            if b["type"] == "tool_use" {
                                let args = serde_json::to_string(&b["input"])
                                    .unwrap_or_else(|_| "{}".into());
                                Some(serde_json::json!({
                                    "id": b["id"],
                                    "type": "function",
                                    "function": { "name": b["name"], "arguments": args },
                                }))
                            } else { None }
                        }).collect();

                        if !tcs.is_empty() {
                            out.push(serde_json::json!({
                                "role": "assistant",
                                "content": null,
                                "tool_calls": tcs,
                            }));
                        } else {
                            out.push(serde_json::json!({
                                "role": "assistant",
                                "content": extract_text_from_blocks(content),
                            }));
                        }
                    }
                }
                "tool" => {
                    if let Some(blocks) = content.as_array() {
                        for b in blocks {
                            if b["type"] == "tool_result" {
                                out.push(serde_json::json!({
                                    "role": "tool",
                                    "tool_call_id": b["tool_use_id"].as_str().unwrap_or("call_0"),
                                    "content": b["content"].as_str().unwrap_or_default(),
                                }));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn oai_tools(tools: &[ApiTool]) -> Vec<serde_json::Value> {
        tools.iter().map(|t| serde_json::json!({
            "type": "function",
            "function": {
                "name": t.name,
                "description": t.description,
                "parameters": t.input_schema,
            },
        })).collect()
    }

    fn build_body(
        &self,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[ApiTool],
        max_tokens: u32,
        stream: bool,
    ) -> serde_json::Value {
        let mut oai_msgs = vec![serde_json::json!({"role": "system", "content": system})];
        oai_msgs.extend(Self::to_openai_messages(messages));

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "messages": oai_msgs,
        });
        if stream {
            body["stream"] = serde_json::json!(true);
        }
        if !tools.is_empty() {
            body["tools"] = serde_json::json!(Self::oai_tools(tools));
            body["tool_choice"] = serde_json::json!("auto");
        }
        body
    }

    async fn send_request(&self, body: &serde_json::Value) -> Result<reqwest::Response, RuntimeError> {
        let resp = self.http.post(self.endpoint())
            .bearer_auth(&self.api_key)
            .json(body)
            .send()
            .await
            .map_err(|e| RuntimeError::Engine(format!("OpenAI HTTP: {e}")))?;

        if !resp.status().is_success() {
            let s = resp.status();
            let b = resp.text().await.unwrap_or_default();
            return Err(RuntimeError::Engine(format!("OpenAI {s}: {b}")));
        }
        Ok(resp)
    }
}

#[async_trait]
impl LlmProvider for OpenAiClient {
    fn default_model(&self) -> &str {
        &self.model
    }

    fn provider_name(&self) -> &str {
        "openai"
    }

    async fn call(
        &self,
        _model: &str,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[ApiTool],
        max_tokens: u32,
    ) -> Result<LlmResponse, RuntimeError> {
        let body = self.build_body(system, messages, tools, max_tokens, false);
        let resp = self.send_request(&body).await?;

        #[derive(Deserialize)]
        struct Choice { message: OaiMsg, finish_reason: String }
        #[derive(Deserialize)]
        struct OaiMsg { content: Option<String>, #[serde(default)] tool_calls: Vec<OaiTc> }
        #[derive(Deserialize)]
        struct OaiTc { id: String, function: OaiFn }
        #[derive(Deserialize)]
        struct OaiFn { name: String, arguments: String }
        #[derive(Deserialize)]
        struct Raw { choices: Vec<Choice> }

        let raw: Raw = resp.json().await
            .map_err(|e| RuntimeError::Engine(format!("OpenAI parse: {e}")))?;
        let choice = raw.choices.into_iter().next()
            .ok_or_else(|| RuntimeError::Engine("OpenAI empty choices".into()))?;

        let stop_reason = if choice.finish_reason == "tool_calls" { "tool_use" } else { "end_turn" };
        let mut content = Vec::new();
        if let Some(text) = choice.message.content.filter(|s| !s.is_empty()) {
            content.push(ContentBlock::Text { text });
        }
        for tc in choice.message.tool_calls {
            let input = serde_json::from_str(&tc.function.arguments).unwrap_or_default();
            content.push(ContentBlock::ToolUse { id: tc.id, name: tc.function.name, input });
        }
        Ok(LlmResponse { stop_reason: stop_reason.into(), content })
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
        let body = self.build_body(system, messages, tools, max_tokens, true);
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
            if data.is_empty() || data == "[DONE]" { break; }
            let val: serde_json::Value = match serde_json::from_str(data) { Ok(v) => v, Err(_) => continue };

            let choice = match val["choices"].get(0) { Some(c) => c, None => continue };
            let delta = &choice["delta"];

            if let Some(text) = delta["content"].as_str() {
                if !text.is_empty() {
                    full_text.push_str(text);
                    on_chunk(StreamChunk::Token(text.to_string()));
                }
            }

            if let Some(tcs) = delta["tool_calls"].as_array() {
                for tc in tcs {
                    let idx = tc["index"].as_u64().unwrap_or(0);
                    let buf = tool_bufs.entry(idx).or_insert_with(|| {
                        (
                            tc["id"].as_str().unwrap_or("").to_string(),
                            tc["function"]["name"].as_str().unwrap_or("").to_string(),
                            String::new(),
                        )
                    });
                    if let Some(args) = tc["function"]["arguments"].as_str() {
                        buf.2.push_str(args);
                    }
                }
            }

            if let Some(reason) = choice["finish_reason"].as_str() {
                if reason == "tool_calls" { stop_reason = "tool_use".to_string(); }
            }
        }

        let mut sorted: Vec<_> = tool_bufs.into_iter().collect();
        sorted.sort_by_key(|(i, _)| *i);
        for (_, (id, name, args)) in sorted {
            let input = serde_json::from_str(&args).unwrap_or_default();
            on_chunk(StreamChunk::ToolUse { id, name, input });
        }

        on_chunk(StreamChunk::Done { stop_reason, full_text });
        Ok(())
    }
}
