use std::time::Duration;

use rune_a2a::A2aClient;

use crate::ToolContext;

// ── Local agent tools (delegate to AgentOps trait) ─────────────────────

pub async fn agent_send(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let ops = ctx
        .agent_ops
        .as_ref()
        .ok_or("Agent operations not configured in this runtime")?;
    let agent_id = input["agent_id"]
        .as_str()
        .ok_or("Missing 'agent_id' parameter")?;
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter")?;
    ops.send_message(agent_id, message).await
}

pub async fn agent_list(ctx: &ToolContext) -> Result<serde_json::Value, String> {
    let ops = ctx
        .agent_ops
        .as_ref()
        .ok_or("Agent operations not configured in this runtime")?;
    ops.list_agents().await
}

pub async fn agent_find(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let ops = ctx
        .agent_ops
        .as_ref()
        .ok_or("Agent operations not configured in this runtime")?;
    let query = input["query"]
        .as_str()
        .ok_or("Missing 'query' parameter")?;
    ops.find_agent(query).await
}

pub async fn agent_spawn(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let ops = ctx
        .agent_ops
        .as_ref()
        .ok_or("Agent operations not configured in this runtime")?;
    let manifest = input["manifest"]
        .as_str()
        .ok_or("Missing 'manifest' parameter")?;
    ops.spawn_agent(manifest).await
}

pub async fn agent_kill(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let ops = ctx
        .agent_ops
        .as_ref()
        .ok_or("Agent operations not configured in this runtime")?;
    let agent_id = input["agent_id"]
        .as_str()
        .ok_or("Missing 'agent_id' parameter")?;
    ops.kill_agent(agent_id).await
}

// ── A2A tools (use rune-a2a client directly) ───────────────────────────

pub async fn a2a_discover(input: &serde_json::Value) -> Result<serde_json::Value, String> {
    let url = input["url"]
        .as_str()
        .ok_or("Missing 'url' parameter")?;

    let client = A2aClient::new(Duration::from_secs(30));
    let card = client
        .discover(url)
        .await
        .map_err(|e| format!("A2A discovery failed: {e}"))?;

    serde_json::to_value(&card).map_err(|e| format!("Failed to serialize agent card: {e}"))
}

pub async fn a2a_send(input: &serde_json::Value) -> Result<serde_json::Value, String> {
    let message_text = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter")?;
    let agent_url = input["agent_url"]
        .as_str()
        .ok_or("Missing 'agent_url' parameter")?;

    let client = A2aClient::new(Duration::from_secs(60));

    let message = rune_a2a::Message {
        message_id: uuid::Uuid::new_v4().to_string(),
        role: rune_a2a::Role::User,
        parts: vec![rune_a2a::Part::text(message_text)],
        ..Default::default()
    };

    let result = client
        .send_message_blocking(agent_url, message)
        .await
        .map_err(|e| format!("A2A send failed: {e}"))?;

    match result {
        rune_a2a::SendMessageResult::Task(task) => {
            let text = task.text_output().unwrap_or_default();
            let data = task.data_output();
            Ok(serde_json::json!({
                "task_id": task.id,
                "state": format!("{:?}", task.status.state),
                "text": text,
                "data": data,
            }))
        }
        rune_a2a::SendMessageResult::Message(msg) => {
            let text_parts: Vec<&str> = msg
                .parts
                .iter()
                .filter_map(|p| {
                    if let rune_a2a::Part::Text { text, .. } = p {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            Ok(serde_json::json!({
                "message_id": msg.message_id,
                "text": text_parts.join("\n"),
            }))
        }
    }
}
