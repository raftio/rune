//! HTTP MCP client — connects to an external MCP server and calls its tools.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;

use crate::error::McpError;
use crate::types::{
    CallToolResult, InitializeResult, JsonRpcRequest, JsonRpcResponse,
    ListToolsResult, McpTool,
};

pub struct McpClient {
    http: reqwest::Client,
    url: String,
    headers: HashMap<String, String>,
}

impl McpClient {
    pub fn new(url: impl Into<String>, headers: HashMap<String, String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");
        Self { http, url: url.into(), headers }
    }

    async fn send(&self, req: JsonRpcRequest) -> Result<Value, McpError> {
        let mut builder = self.http.post(&self.url).json(&req);
        for (k, v) in &self.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        let resp = builder.send().await?;
        let rpc: JsonRpcResponse = resp.json().await?;
        if let Some(e) = rpc.error {
            return Err(McpError::Protocol { code: e.code, message: e.message });
        }
        Ok(rpc.result.unwrap_or(Value::Null))
    }

    /// Perform the MCP handshake.
    pub async fn initialize(&self) -> Result<InitializeResult, McpError> {
        let req = JsonRpcRequest::new("initialize", serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "rune", "version": "0.1.0" }
        }));
        let result = self.send(req).await?;
        serde_json::from_value(result).map_err(McpError::Json)
    }

    /// List all tools exposed by this MCP server.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        let req = JsonRpcRequest::new("tools/list", Value::Null);
        let result = self.send(req).await?;
        let list: ListToolsResult = serde_json::from_value(result).map_err(McpError::Json)?;
        Ok(list.tools)
    }

    /// Call a tool by name with the given arguments.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
    ) -> Result<CallToolResult, McpError> {
        let req = JsonRpcRequest::new("tools/call", serde_json::json!({
            "name": name,
            "arguments": arguments,
        }));
        let result = self.send(req).await?;
        serde_json::from_value(result).map_err(McpError::Json)
    }
}
