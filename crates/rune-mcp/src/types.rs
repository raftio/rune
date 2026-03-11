use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── JSON-RPC ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl JsonRpcRequest {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id: Some(Value::String(uuid::Uuid::new_v4().to_string())),
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    pub fn invalid_request(msg: impl Into<String>) -> Self {
        Self { code: -32600, message: msg.into(), data: None }
    }
    pub fn method_not_found(method: &str) -> Self {
        Self { code: -32601, message: format!("Method not found: {method}"), data: None }
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self { code: -32603, message: msg.into(), data: None }
    }
}

// ── MCP protocol types ───────────────────────────────────────────────────────

/// An MCP tool as returned by `tools/list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub input_schema: Value,
}

/// Content block in a tool call result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolContent {
    #[serde(rename = "type")]
    pub content_type: String,  // "text" | "image" | "resource"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ToolContent {
    pub fn text(s: impl Into<String>) -> Self {
        Self { content_type: "text".into(), text: Some(s.into()), data: None }
    }
    pub fn data(v: Value) -> Self {
        Self { content_type: "text".into(), text: Some(v.to_string()), data: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    pub content: Vec<ToolContent>,
    #[serde(default)]
    pub is_error: bool,
}

impl CallToolResult {
    pub fn ok(text: impl Into<String>) -> Self {
        Self { content: vec![ToolContent::text(text)], is_error: false }
    }
    pub fn ok_json(v: Value) -> Self {
        Self { content: vec![ToolContent::data(v)], is_error: false }
    }
    pub fn error(msg: impl Into<String>) -> Self {
        Self { content: vec![ToolContent::text(msg)], is_error: true }
    }
    /// Extract the text/json output as a serde_json::Value.
    pub fn to_value(&self) -> Value {
        let texts: Vec<&str> = self.content.iter()
            .filter_map(|c| c.text.as_deref())
            .collect();
        let combined = texts.join("\n");
        // Try to parse as JSON; fall back to plain text
        serde_json::from_str(&combined).unwrap_or(Value::String(combined))
    }
}

/// Response from `tools/list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListToolsResult {
    pub tools: Vec<McpTool>,
}

/// MCP server info from `initialize`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: Value,
    pub server_info: ServerInfo,
}

/// Configuration for an external MCP server referenced from a Runefile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Logical name used as prefix for tools: `{name}/{tool}`.
    pub name: String,
    /// HTTP URL of the MCP server endpoint.
    pub url: String,
    /// Optional HTTP headers (supports `${ENV_VAR}` interpolation).
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
}
