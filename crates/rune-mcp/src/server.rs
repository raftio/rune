//! MCP server handler — exposes Rune as an MCP server.
//!
//! POST /mcp
//!   tools/list  → list all deployed agents (each agent is an MCP tool)
//!   tools/call  → invoke an agent via the gateway invoke endpoint

use serde_json::Value;

use crate::types::{
    CallToolResult, InitializeResult, JsonRpcError, JsonRpcRequest,
    JsonRpcResponse, ListToolsResult, McpTool, ServerInfo, ToolContent,
};

pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Handle a single MCP JSON-RPC request.
/// `agent_tools` is the list of agent names (or tool descriptors) to expose.
/// `invoke_fn` is called when `tools/call` fires — receives (tool_name, arguments)
/// and returns the text result.
pub async fn handle_request<F, Fut>(
    rpc: JsonRpcRequest,
    agent_tools: Vec<McpTool>,
    invoke_fn: F,
) -> JsonRpcResponse
where
    F: FnOnce(String, Value) -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    let id = rpc.id.clone();
    let result = dispatch(rpc, agent_tools, invoke_fn).await;
    match result {
        Ok(v) => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(v),
            error: None,
        },
        Err(e) => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(e),
        },
    }
}

async fn dispatch<F, Fut>(
    rpc: JsonRpcRequest,
    agent_tools: Vec<McpTool>,
    invoke_fn: F,
) -> Result<Value, JsonRpcError>
where
    F: FnOnce(String, Value) -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    match rpc.method.as_str() {
        "initialize" | "notifications/initialized" => {
            let result = InitializeResult {
                protocol_version: MCP_PROTOCOL_VERSION.into(),
                capabilities: serde_json::json!({ "tools": {} }),
                server_info: ServerInfo {
                    name: "rune".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                },
            };
            serde_json::to_value(result).map_err(|e| JsonRpcError::internal(e.to_string()))
        }

        "tools/list" => {
            let list = ListToolsResult { tools: agent_tools };
            serde_json::to_value(list).map_err(|e| JsonRpcError::internal(e.to_string()))
        }

        "tools/call" => {
            let name = rpc.params["name"]
                .as_str()
                .ok_or_else(|| JsonRpcError::invalid_request("missing tool name"))?
                .to_string();
            let arguments = rpc.params["arguments"].clone();

            match invoke_fn(name, arguments).await {
                Ok(v) => {
                    let res = CallToolResult {
                        content: vec![ToolContent::data(v)],
                        is_error: false,
                    };
                    serde_json::to_value(res).map_err(|e| JsonRpcError::internal(e.to_string()))
                }
                Err(e) => {
                    let res = CallToolResult::error(e);
                    serde_json::to_value(res).map_err(|e2| JsonRpcError::internal(e2.to_string()))
                }
            }
        }

        "ping" => Ok(serde_json::json!({})),

        _ => Err(JsonRpcError::method_not_found(&rpc.method)),
    }
}
