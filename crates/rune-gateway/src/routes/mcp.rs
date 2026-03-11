//! MCP server endpoint — exposes all deployed Rune agents as MCP tools.
//!
//! POST /mcp   — MCP JSON-RPC (initialize, tools/list, tools/call)

use std::time::Duration;

use axum::{extract::State, response::IntoResponse, Json};
use serde_json::Value;

use rune_mcp::types::{JsonRpcRequest, McpTool};

use crate::AppState;

pub async fn handle_mcp(
    State(state): State<AppState>,
    Json(rpc): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    let agent_tools = build_agent_tools(&state).await.unwrap_or_default();
    let gateway_url = state.env.gateway_base_url.clone();
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_default();

    let response = rune_mcp::server::handle_request(
        rpc,
        agent_tools,
        move |tool_name, arguments| {
            let http = http.clone();
            let gateway_url = gateway_url.clone();
            async move {
                let input = arguments.get("input")
                    .cloned()
                    .unwrap_or_else(|| Value::String(arguments.to_string()));
                let session_id = arguments.get("session_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let mut body = serde_json::json!({ "input": input });
                if let Some(sid) = session_id {
                    body["session_id"] = Value::String(sid);
                }

                let url = format!("{}/v1/agents/{}/invoke",
                    gateway_url.trim_end_matches('/'),
                    tool_name);

                http.post(&url)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?
                    .json::<Value>()
                    .await
                    .map_err(|e| e.to_string())
            }
        },
    )
    .await;

    Json(response)
}

async fn build_agent_tools(state: &AppState) -> Result<Vec<McpTool>, anyhow::Error> {
    let names = state.store.list_deployed_agent_names().await?;
    let tools = names
        .into_iter()
        .map(|name| McpTool {
            description: format!("Rune agent '{}'", name),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "The message or prompt to send to this agent"
                    },
                    "session_id": {
                        "type": "string",
                        "description": "Optional UUID for multi-turn conversation sessions"
                    }
                },
                "required": ["input"]
            }),
            name,
        })
        .collect();
    Ok(tools)
}
