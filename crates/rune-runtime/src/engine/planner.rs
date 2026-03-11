use uuid::Uuid;

use crate::error::RuntimeError;
use crate::metrics;

use super::llm::{ContentBlock, LlmClient, tools_to_api, user_input_to_content};
use super::policy::{PolicyDecision, PolicyEngine};
use super::session::{Message, SessionManager};
use super::tool_dispatcher::ToolDispatcher;
use super::loader::ExecutionPlan;

// ---------------------------------------------------------------------------
// Action
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum Action {
    Respond { content: serde_json::Value },
    CallTool { id: String, name: String, input: serde_json::Value },
    Finish { reason: String },
}

// ---------------------------------------------------------------------------
// Planner
// ---------------------------------------------------------------------------

pub struct Planner {
    plan: ExecutionPlan,
    /// Pre-built model policy — computed once from plan.models to avoid per-step allocation.
    model_policy: PolicyEngine,
}

impl Planner {
    pub fn new(plan: ExecutionPlan) -> Self {
        let model_policy = PolicyEngine::new(plan.toolset.clone().into_iter(), plan.models.clone());
        Self { plan, model_policy }
    }

    /// Convert stored session messages to Anthropic API message format
    /// (used as the internal wire format for all providers).
    fn to_api_messages(messages: &[Message]) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for msg in messages {
            match msg.role.as_str() {
                "user" => {
                    let content = user_input_to_content(&msg.content);
                    out.push(serde_json::json!({ "role": "user", "content": content }));
                }
                "assistant" => {
                    let content = if msg.content.is_array() {
                        msg.content.clone()
                    } else {
                        serde_json::json!([{ "type": "text", "text": msg.content.to_string() }])
                    };
                    out.push(serde_json::json!({ "role": "assistant", "content": content }));
                }
                "tool" => {
                    // Tool results stored as "tool" role; provider clients map appropriately.
                    let content = if msg.content.is_array() {
                        msg.content.clone()
                    } else {
                        serde_json::json!([msg.content])
                    };
                    out.push(serde_json::json!({ "role": "tool", "content": content }));
                }
                _ => {}
            }
        }
        out
    }

    async fn next_action(
        &self,
        client: &LlmClient,
        messages: &[Message],
        step: u32,
    ) -> Result<Action, RuntimeError> {
        if step >= self.plan.max_steps {
            return Ok(Action::Finish {
                reason: format!("max_steps ({}) reached", self.plan.max_steps),
            });
        }

        if let PolicyDecision::Deny { reason } = self.model_policy.check_model(self.plan.default_model.as_str()) {
            return Err(RuntimeError::Engine(format!("model policy denied: {reason}")));
        }

        let api_messages = Self::to_api_messages(messages);
        let api_tools = tools_to_api(&self.plan.tools);

        let resp = client
            .call(
                Some(&self.plan.default_model),
                &self.plan.instructions,
                &api_messages,
                &api_tools,
                4096,
            )
            .await?;

        match resp.stop_reason.as_str() {
            "tool_use" => {
                for block in &resp.content {
                    if let ContentBlock::ToolUse { id, name, input } = block {
                        return Ok(Action::CallTool {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        });
                    }
                }
                Ok(Action::Finish {
                    reason: "stop_reason=tool_use but no tool_use block found".into(),
                })
            }
            _ => {
                let text = resp.content.iter()
                    .filter_map(|b| if let ContentBlock::Text { text } = b { Some(text.as_str()) } else { None })
                    .collect::<Vec<_>>()
                    .join("\n");

                let content_blocks = serde_json::to_value(&resp.content)
                    .map_err(|e| RuntimeError::Engine(e.to_string()))?;

                Ok(Action::Respond {
                    content: serde_json::json!({ "text": text, "_blocks": content_blocks }),
                })
            }
        }
    }

    /// NOTE: this method is NOT safe for concurrent calls on the same session_id.
    /// The caller (route handler) MUST hold an exclusive replica lease for the
    /// session before entering this function.  If two requests share a session
    /// without coordination, messages may be duplicated or lost.
    pub async fn run(
        &self,
        client: &LlmClient,
        sessions: &SessionManager,
        session_id: Uuid,
        user_input: serde_json::Value,
        tools: &ToolDispatcher,
        request_id: Uuid,
        store: &rune_storage::RuneStore,
    ) -> Result<serde_json::Value, RuntimeError> {
        let mut messages = sessions.load_messages(session_id).await?;
        let step = messages.len() as i64;
        let user_content = user_input_to_content(&user_input);
        sessions.append_message(session_id, "user", user_content, step).await?;
        messages = sessions.load_messages(session_id).await?;

        let mut current_step: u32 = 0;
        loop {
            match self.next_action(client, &messages, current_step).await? {
                Action::Respond { content } => {
                    let blocks = content.get("_blocks").cloned()
                        .unwrap_or(serde_json::json!([{"type":"text","text": content.to_string()}]));
                    sessions
                        .append_message(session_id, "assistant", blocks, messages.len() as i64)
                        .await?;
                    messages = sessions.load_messages(session_id).await?;
                    let step = messages.len() as i64;
                    if let Err(e) = sessions.checkpoint(session_id, &messages, step).await {
                        tracing::error!(session_id = %session_id, error = %e, "checkpoint failed — session history may be incomplete");
                    }
                    let text = content.get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    return Ok(serde_json::json!({ "text": text }));
                }

                Action::CallTool { id, name, input } => {
                    let assistant_blocks = serde_json::json!([{
                        "type": "tool_use", "id": id, "name": name, "input": input,
                    }]);
                    sessions
                        .append_message(session_id, "assistant", assistant_blocks, messages.len() as i64)
                        .await?;

                    let result = tools.dispatch(&name, input, request_id, store).await?;

                    let tool_result = serde_json::json!([{
                        "type": "tool_result",
                        "tool_use_id": id,
                        "content": serde_json::to_string(&result).unwrap_or_default(),
                    }]);
                    messages = sessions.load_messages(session_id).await?;
                    sessions
                        .append_message(session_id, "tool", tool_result, messages.len() as i64)
                        .await?;

                    messages = sessions.load_messages(session_id).await?;
                    current_step += 1;
                }

                Action::Finish { reason } => {
                    let step = messages.len() as i64;
                    if let Err(e) = sessions.checkpoint(session_id, &messages, step).await {
                        tracing::error!(session_id = %session_id, error = %e, "checkpoint failed at finish — session history may be incomplete");
                    }
                    return Ok(serde_json::json!({ "finished": true, "reason": reason }));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Stub planner (no API key configured)
// ---------------------------------------------------------------------------

pub struct StubPlanner {
    agent_name: String,
}

impl StubPlanner {
    pub fn new(agent_name: impl Into<String>) -> Self {
        Self { agent_name: agent_name.into() }
    }

    pub async fn run(
        &self,
        sessions: &SessionManager,
        session_id: Uuid,
        user_input: serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        let step = sessions.load_messages(session_id).await?.len() as i64;
        sessions.append_message(session_id, "user", user_input.clone(), step).await?;
        let reply = serde_json::json!({
            "text": format!("[stub] echo from {}: {}", self.agent_name, user_input),
            "note": "Set ANTHROPIC_API_KEY or OPENAI_API_KEY to enable real LLM responses"
        });
        sessions.append_message(session_id, "assistant", reply.clone(), step + 1).await?;
        Ok(reply)
    }
}

// ---------------------------------------------------------------------------
// SSE event (client-facing wire format)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SseEvent {
    Token { text: String },
    ToolStart { name: String },
    ToolDone { name: String, result: serde_json::Value },
    Done { session_id: String, request_id: String },
    Error { message: String },
}

use serde::Serialize;
use super::llm::StreamChunk;

#[cfg(test)]
mod tests {
    use super::*;
    use rune_storage::SessionMessage;

    fn make_message(role: &str, content: serde_json::Value) -> SessionMessage {
        SessionMessage {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: uuid::Uuid::new_v4().to_string(),
            role: role.into(),
            content,
            step: 0,
            created_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    // --- to_api_messages ---

    #[test]
    fn user_message_maps_to_user_role_with_content_array() {
        let msg = make_message("user", serde_json::json!("hello"));
        let out = Planner::to_api_messages(&[msg]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "user");
        assert!(out[0]["content"].is_array());
    }

    #[test]
    fn assistant_text_message_wrapped_in_array() {
        let msg = make_message("assistant", serde_json::json!("I can help"));
        let out = Planner::to_api_messages(&[msg]);
        assert_eq!(out[0]["role"], "assistant");
        assert!(out[0]["content"].is_array());
    }

    #[test]
    fn assistant_array_content_passes_through() {
        let content = serde_json::json!([{"type": "text", "text": "reply"}]);
        let msg = make_message("assistant", content.clone());
        let out = Planner::to_api_messages(&[msg]);
        assert_eq!(out[0]["content"], content);
    }

    #[test]
    fn tool_message_maps_to_tool_role() {
        let content = serde_json::json!([{"type": "tool_result", "tool_use_id": "id1", "content": "ok"}]);
        let msg = make_message("tool", content);
        let out = Planner::to_api_messages(&[msg]);
        assert_eq!(out[0]["role"], "tool");
        assert!(out[0]["content"].is_array());
    }

    #[test]
    fn tool_non_array_content_gets_wrapped() {
        let msg = make_message("tool", serde_json::json!({"result": "done"}));
        let out = Planner::to_api_messages(&[msg]);
        assert!(out[0]["content"].is_array());
    }

    #[test]
    fn unknown_role_is_skipped() {
        let msg = make_message("system", serde_json::json!("ignored"));
        let out = Planner::to_api_messages(&[msg]);
        assert!(out.is_empty());
    }

    #[test]
    fn multiple_messages_preserved_in_order() {
        let msgs = vec![
            make_message("user", serde_json::json!("first")),
            make_message("assistant", serde_json::json!("second")),
            make_message("tool", serde_json::json!([{"type":"tool_result","tool_use_id":"x","content":"ok"}])),
        ];
        let out = Planner::to_api_messages(&msgs);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0]["role"], "user");
        assert_eq!(out[1]["role"], "assistant");
        assert_eq!(out[2]["role"], "tool");
    }

    #[test]
    fn empty_messages_returns_empty() {
        let out = Planner::to_api_messages(&[]);
        assert!(out.is_empty());
    }

    // --- SseEvent serialization ---

    #[test]
    fn sse_token_serializes_with_type_field() {
        let ev = SseEvent::Token { text: "hello".into() };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "token");
        assert_eq!(json["text"], "hello");
    }

    #[test]
    fn sse_tool_start_serializes_correctly() {
        let ev = SseEvent::ToolStart { name: "rune__shell".into() };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "tool_start");
        assert_eq!(json["name"], "rune__shell");
    }

    #[test]
    fn sse_tool_done_serializes_correctly() {
        let ev = SseEvent::ToolDone {
            name: "my_tool".into(),
            result: serde_json::json!({"output": "done"}),
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "tool_done");
        assert_eq!(json["name"], "my_tool");
        assert_eq!(json["result"]["output"], "done");
    }

    #[test]
    fn sse_done_serializes_correctly() {
        let ev = SseEvent::Done {
            session_id: "sess-1".into(),
            request_id: "req-1".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "done");
        assert_eq!(json["session_id"], "sess-1");
        assert_eq!(json["request_id"], "req-1");
    }

    #[test]
    fn sse_error_serializes_correctly() {
        let ev = SseEvent::Error { message: "something failed".into() };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "error");
        assert_eq!(json["message"], "something failed");
    }

    // --- StubPlanner ---

    #[test]
    fn stub_planner_stores_agent_name() {
        let planner = StubPlanner::new("my-agent");
        let _ = planner; // construction succeeds
    }

    #[test]
    fn stub_planner_accepts_string_and_str() {
        let _ = StubPlanner::new("str-name");
        let _ = StubPlanner::new("string-name".to_string());
    }
}


impl Planner {
    /// Streaming version of `run()`. Sends `SseEvent`s to `tx` as tokens arrive.
    pub async fn run_stream(
        &self,
        client: &LlmClient,
        sessions: &SessionManager,
        session_id: Uuid,
        user_input: serde_json::Value,
        tools: &ToolDispatcher,
        request_id: Uuid,
        store: &rune_storage::RuneStore,
        tx: tokio::sync::mpsc::Sender<SseEvent>,
    ) -> Result<(), RuntimeError> {
        // Append user turn.
        let mut messages = sessions.load_messages(session_id).await?;
        sessions
            .append_message(session_id, "user", user_input_to_content(&user_input), messages.len() as i64)
            .await?;
        messages = sessions.load_messages(session_id).await?;

        let api_tools = tools_to_api(&self.plan.tools);
        let mut current_step: u32 = 0;

        loop {
            if current_step >= self.plan.max_steps {
                metrics::increment_streaming_events();
                let _ = tx.send(SseEvent::Done {
                    session_id: session_id.to_string(),
                    request_id: request_id.to_string(),
                }).await;
                return Ok(());
            }

            if let PolicyDecision::Deny { reason } =
                self.model_policy.check_model(self.plan.default_model.as_str())
            {
                let _ = tx.send(SseEvent::Error {
                    message: format!("model policy denied: {reason}"),
                }).await;
                return Ok(());
            }

            let api_messages = Self::to_api_messages(&messages);

            // Collect chunks via synchronous callback — avoids spawning or lifetime issues.
            let mut full_text = String::new();
            let mut tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            let mut stop_reason = "end_turn".to_string();
            let mut stream_err: Option<String> = None;

            client.stream(
                Some(&self.plan.default_model),
                &self.plan.instructions,
                &api_messages,
                &api_tools,
                4096,
                &mut |chunk| {
                    match chunk {
                        StreamChunk::Token(text) => {
                            full_text.push_str(&text);
                            metrics::increment_streaming_events();
                            // try_send: drop token if SSE buffer full (non-fatal)
                            let _ = tx.try_send(SseEvent::Token { text });
                        }
                        StreamChunk::ToolUse { id, name, input } => {
                            tool_calls.push((id, name, input));
                        }
                        StreamChunk::Done { stop_reason: sr, full_text: ft } => {
                            stop_reason = sr;
                            if full_text.is_empty() { full_text = ft; }
                        }
                        StreamChunk::Error(msg) => {
                            let _ = tx.try_send(SseEvent::Error { message: msg.clone() });
                            stream_err = Some(msg);
                        }
                    }
                },
            ).await?;

            if stream_err.is_some() {
                return Ok(());
            }

            if stop_reason == "tool_use" && !tool_calls.is_empty() {
                // Store assistant turn with tool_use blocks.
                let blocks: Vec<_> = tool_calls.iter().map(|(id, name, input)| {
                    serde_json::json!({"type":"tool_use","id":id,"name":name,"input":input})
                }).collect();
                sessions
                    .append_message(session_id, "assistant", serde_json::json!(blocks), messages.len() as i64)
                    .await?;

                for (id, name, input) in &tool_calls {
                    metrics::increment_streaming_events();
                    let _ = tx.send(SseEvent::ToolStart { name: name.clone() }).await;
                    let result = tools.dispatch(name, input.clone(), request_id, store).await?;
                    metrics::increment_streaming_events();
                    let _ = tx.send(SseEvent::ToolDone { name: name.clone(), result: result.clone() }).await;

                    let tool_result = serde_json::json!([{
                        "type": "tool_result",
                        "tool_use_id": id,
                        "content": serde_json::to_string(&result).unwrap_or_default(),
                    }]);
                    messages = sessions.load_messages(session_id).await?;
                    sessions
                        .append_message(session_id, "tool", tool_result, messages.len() as i64)
                        .await?;
                }

                messages = sessions.load_messages(session_id).await?;
                current_step += 1;
            } else {
                let blocks = serde_json::json!([{"type":"text","text": full_text}]);
                sessions
                    .append_message(session_id, "assistant", blocks, messages.len() as i64)
                    .await?;
                messages = sessions.load_messages(session_id).await?;
                let step = messages.len() as i64;
                if let Err(e) = sessions.checkpoint(session_id, &messages, step).await {
                    tracing::error!(session_id = %session_id, error = %e, "checkpoint failed in stream — session history may be incomplete");
                }
                metrics::increment_streaming_events();
                let _ = tx.send(SseEvent::Done {
                    session_id: session_id.to_string(),
                    request_id: request_id.to_string(),
                }).await;
                return Ok(());
            }
        }
    }
}
