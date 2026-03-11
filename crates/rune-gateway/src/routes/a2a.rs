use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use rune_storage::{RuneStore, RuntimeStore};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt as _;
use uuid::Uuid;

use rune_a2a::jsonrpc::*;
use rune_a2a::types::*;

use crate::error::GatewayError;
use rune_runtime::{LlmClient, Planner, ReplicaRouter, SessionManager, SseEvent, StubPlanner, ToolDispatcher};

// ---------------------------------------------------------------------------
// Agent Card endpoints
// ---------------------------------------------------------------------------

pub async fn agent_card_global(
    State(state): State<crate::AppState>,
) -> Result<Json<AgentCard>, GatewayError> {
    let base_url = gateway_base_url(&state);

    let agent_names = state.store
        .list_deployed_agent_names()
        .await
        .map_err(rune_runtime::RuntimeError::Storage)?;

    let skills: Vec<AgentSkill> = agent_names
        .iter()
        .map(|name| AgentSkill {
            id: name.clone(),
            name: name.clone(),
            description: format!("Agent: {name}"),
            tags: vec!["agent".into()],
            examples: vec![],
            input_modes: None,
            output_modes: None,
        })
        .collect();

    let card = AgentCard {
        name: "Rune Agent Runtime".into(),
        description: "A2A-compliant agent runtime powered by Rune".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        supported_interfaces: vec![AgentInterface {
            url: format!("{base_url}/a2a"),
            protocol_binding: "JSONRPC".into(),
            protocol_version: "1.0".into(),
            tenant: None,
        }],
        capabilities: AgentCapabilities {
            streaming: true,
            push_notifications: false,
            extended_agent_card: false,
        },
        default_input_modes: vec!["text/plain".into(), "application/json".into()],
        default_output_modes: vec!["text/plain".into(), "application/json".into()],
        skills,
        provider: None,
        documentation_url: None,
        icon_url: None,
    };

    Ok(Json(card))
}

pub async fn agent_card(
    State(state): State<crate::AppState>,
    Path(agent_name): Path<String>,
) -> Result<Json<AgentCard>, GatewayError> {
    let base_url = gateway_base_url(&state);

    if !state.store.verify_agent_deployed(&agent_name).await.map_err(rune_runtime::RuntimeError::Storage)? {
        return Err(GatewayError::NotFound(format!(
            "no active deployment for agent '{agent_name}'"
        )));
    }

    let plan = crate::routes::resolve_plan(&agent_name, state.env.agent_packages_dir.as_deref());

    let skills: Vec<AgentSkill> = plan
        .tools
        .iter()
        .map(|t| AgentSkill {
            id: t.name.clone(),
            name: t.name.clone(),
            description: format!("Tool: {}", t.name),
            tags: t.capabilities.clone(),
            examples: vec![],
            input_modes: None,
            output_modes: None,
        })
        .collect();

    let card = AgentCard {
        name: plan.agent_name.clone(),
        description: first_line(&plan.instructions),
        version: "0.1.0".into(),
        supported_interfaces: vec![AgentInterface {
            url: format!("{base_url}/a2a/{agent_name}"),
            protocol_binding: "JSONRPC".into(),
            protocol_version: "1.0".into(),
            tenant: None,
        }],
        capabilities: AgentCapabilities {
            streaming: true,
            push_notifications: false,
            extended_agent_card: false,
        },
        default_input_modes: vec!["text/plain".into(), "application/json".into()],
        default_output_modes: vec!["text/plain".into(), "application/json".into()],
        skills,
        provider: None,
        documentation_url: None,
        icon_url: None,
    };

    Ok(Json(card))
}

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 endpoint
// ---------------------------------------------------------------------------

pub async fn jsonrpc_handler(
    State(state): State<crate::AppState>,
    Path(agent_name): Path<String>,
    Json(rpc_req): Json<JsonRpcRequest>,
) -> Result<Response, GatewayError> {
    let rpc_id = rpc_req.id.clone();

    match rpc_req.method.as_str() {
        METHOD_MESSAGE_SEND => {
            handle_message_send(state.clone(), &agent_name, rpc_req).await
        }
        METHOD_MESSAGE_STREAM => {
            handle_message_stream(state.clone(), &agent_name, rpc_req).await
        }
        METHOD_TASKS_GET => {
            handle_tasks_get(state.store.clone(), rpc_req).await
        }
        METHOD_TASKS_CANCEL => {
            handle_tasks_cancel(state.store.clone(), rpc_req).await
        }
        _ => {
            let resp = JsonRpcResponse::error(
                rpc_id,
                JsonRpcError {
                    code: METHOD_NOT_FOUND,
                    message: format!("unknown method: {}", rpc_req.method),
                    data: None,
                },
            );
            Ok(Json(resp).into_response())
        }
    }
}

// ---------------------------------------------------------------------------
// message/send
// ---------------------------------------------------------------------------

const MAX_A2A_CALL_DEPTH: u32 = 10;

async fn handle_message_send(
    state: crate::AppState,
    agent_name: &str,
    rpc_req: JsonRpcRequest,
) -> Result<Response, GatewayError> {
    let store = state.store.clone();
    let env = &state.env;
    let rpc_id = rpc_req.id.clone();

    let params: SendMessageParams = serde_json::from_value(rpc_req.params)
        .map_err(|e| GatewayError::Internal(format!("invalid params: {e}")))?;

    let incoming_depth = extract_call_depth(&params.message);
    if incoming_depth >= MAX_A2A_CALL_DEPTH {
        let resp = JsonRpcResponse::error(
            rpc_id,
            JsonRpcError {
                code: INVALID_PARAMS,
                message: format!(
                    "agent call depth {} exceeds maximum {}",
                    incoming_depth, MAX_A2A_CALL_DEPTH
                ),
                data: None,
            },
        );
        return Ok(Json(resp).into_response());
    }

    let deployment_id = resolve_deployment(&store, agent_name).await?;
    let sessions = SessionManager::new(store.clone());

    let (session_id, context_id) = resolve_or_create_session(
        &sessions,
        deployment_id,
        params.message.context_id.as_deref(),
    )
    .await?;

    let router = ReplicaRouter::new(store.clone());
    let lease = router.acquire(deployment_id, session_id).await
        .map_err(|_| GatewayError::NoReplicaAvailable(deployment_id.to_string()))?;
    let replica_id = lease.replica_id;

    let plan = crate::routes::resolve_plan(agent_name, env.agent_packages_dir.as_deref());
    let tool_ctx = crate::routes::shared_tool_context(&store, env, Some(agent_name));
    let tools = ToolDispatcher::with_depth(plan.tools.clone(), plan.agent_dir.clone(), incoming_depth)
        .with_tool_context(tool_ctx)
        .with_caller_networks(plan.networks.clone());

    let user_input = parts_to_input(&params.message.parts);

    let request_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();

    store
        .insert_request(request_id, session_id, deployment_id, Some(replica_id), &user_input)
        .await
        .map_err(rune_runtime::RuntimeError::RuntimeStore)?;

    store
        .insert_a2a_task(&task_id.to_string(), &context_id, &request_id.to_string(), &session_id.to_string(), agent_name)
        .await
        .map_err(rune_runtime::RuntimeError::RuntimeStore)?;

    let now_ts = chrono::Utc::now().to_rfc3339();

    let output = if let Some(client) = LlmClient::from_env() {
        let planner = Planner::new(plan);
        planner
            .run(&client, &sessions, session_id, user_input, &tools, request_id, &store)
            .await?
    } else {
        let stub = StubPlanner::new(agent_name);
        stub.run(&sessions, session_id, user_input).await?
    };

    lease.release().await;

    store.update_request_completed(request_id, &output).await.map_err(rune_runtime::RuntimeError::RuntimeStore)?;
    store.update_a2a_task_state(&task_id.to_string(), "completed").await.map_err(rune_runtime::RuntimeError::RuntimeStore)?;

    let output_text = output
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let task = Task {
        id: task_id.to_string(),
        context_id: Some(context_id),
        status: TaskStatus {
            state: TaskState::Completed,
            message: None,
            timestamp: Some(now_ts),
        },
        artifacts: Some(vec![Artifact {
            artifact_id: Uuid::new_v4().to_string(),
            name: Some("response".into()),
            description: None,
            parts: vec![Part::text(output_text)],
            metadata: None,
        }]),
        history: Some(vec![params.message]),
        metadata: None,
        kind: "task".into(),
    };

    let result = serde_json::to_value(&task)
        .map_err(|e| GatewayError::Internal(e.to_string()))?;
    let resp = JsonRpcResponse::success(rpc_id, result);
    Ok(Json(resp).into_response())
}

// ---------------------------------------------------------------------------
// message/stream
// ---------------------------------------------------------------------------

async fn handle_message_stream(
    state: crate::AppState,
    agent_name: &str,
    rpc_req: JsonRpcRequest,
) -> Result<Response, GatewayError> {
    let store = state.store.clone();
    let env = &state.env;
    let params: SendMessageParams = serde_json::from_value(rpc_req.params)
        .map_err(|e| GatewayError::Internal(format!("invalid params: {e}")))?;

    let incoming_depth = extract_call_depth(&params.message);
    if incoming_depth >= MAX_A2A_CALL_DEPTH {
        let resp = JsonRpcResponse::error(
            rpc_req.id.clone(),
            JsonRpcError {
                code: INVALID_PARAMS,
                message: format!(
                    "agent call depth {} exceeds maximum {}",
                    incoming_depth, MAX_A2A_CALL_DEPTH
                ),
                data: None,
            },
        );
        return Ok(Json(resp).into_response());
    }

    let deployment_id = resolve_deployment(&store, agent_name).await?;
    let sessions = SessionManager::new(store.clone());
    let rpc_id = rpc_req.id.clone();

    let (session_id, context_id) = resolve_or_create_session(
        &sessions,
        deployment_id,
        params.message.context_id.as_deref(),
    )
    .await?;

    let router = ReplicaRouter::new(store.clone());
    let lease = router.acquire(deployment_id, session_id).await
        .map_err(|_| GatewayError::NoReplicaAvailable(deployment_id.to_string()))?;
    let replica_id = lease.replica_id;

    let plan = crate::routes::resolve_plan(agent_name, env.agent_packages_dir.as_deref());
    let tool_ctx = crate::routes::shared_tool_context(&store, env, Some(agent_name));
    let tools = ToolDispatcher::with_depth(plan.tools.clone(), plan.agent_dir.clone(), incoming_depth)
        .with_tool_context(tool_ctx)
        .with_caller_networks(plan.networks.clone());
    let user_input = parts_to_input(&params.message.parts);

    let request_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();

    store.insert_request(request_id, session_id, deployment_id, Some(replica_id), &user_input).await.map_err(rune_runtime::RuntimeError::RuntimeStore)?;
    store.insert_a2a_task(&task_id.to_string(), &context_id, &request_id.to_string(), &session_id.to_string(), agent_name).await.map_err(rune_runtime::RuntimeError::RuntimeStore)?;

    let Some(client) = LlmClient::from_env() else {
        return Err(GatewayError::Internal(
            "streaming requires ANTHROPIC_API_KEY or OPENAI_API_KEY".into(),
        ));
    };

    let (sse_tx, sse_rx) = mpsc::channel::<SseEvent>(64);
    let (a2a_tx, a2a_rx) = mpsc::channel::<serde_json::Value>(64);

    let task_id_str = task_id.to_string();
    let context_id_clone = context_id.clone();
    let rpc_id_clone = rpc_id.clone();
    let agent_name_owned = agent_name.to_string();
    let store2 = store.clone();

    let initial_task = Task {
        id: task_id_str.clone(),
        context_id: Some(context_id.clone()),
        status: TaskStatus {
            state: TaskState::Working,
            message: None,
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        },
        artifacts: None,
        history: Some(vec![params.message.clone()]),
        metadata: None,
        kind: "task".into(),
    };
    let initial_result = serde_json::to_value(&initial_task)
        .map_err(|e| GatewayError::Internal(format!("failed to serialize initial task: {e}")))?;
    let initial_val = JsonRpcResponse::success(rpc_id_clone.clone(), initial_result);
    let initial_payload = serde_json::to_value(&initial_val)
        .map_err(|e| GatewayError::Internal(format!("failed to serialize initial response: {e}")))?;
    let _ = a2a_tx.send(initial_payload).await;

    tokio::spawn(async move {
        let planner = Planner::new(plan);
        let result = planner
            .run_stream(
                &client,
                &sessions,
                session_id,
                user_input,
                &tools,
                request_id,
                &store2,
                sse_tx,
            )
            .await;

        let status = if result.is_ok() { "completed" } else { "failed" };
        if let Err(e) = store2.update_request_status(request_id, status).await {
            tracing::error!(
                request_id = %request_id,
                error = %e,
                "failed to persist request status after A2A stream"
            );
        }
        if let Err(e) = store2.update_a2a_task_state(&task_id_str, status).await {
            tracing::error!(
                task_id = %task_id_str,
                error = %e,
                "failed to persist A2A task state after stream"
            );
        }

        if let Err(ref e) = result {
            tracing::error!(
                agent = agent_name_owned,
                request_id = %request_id,
                "stream planner error: {e}"
            );
        }

        lease.release().await;
    });

    let context_for_bridge = context_id_clone;
    let task_for_bridge = task_id.to_string();
    let rpc_for_bridge = rpc_id;

    tokio::spawn(async move {
        let mut sse_rx = sse_rx;
        let mut full_text = String::new();

        while let Some(event) = sse_rx.recv().await {
            let (a2a_event, is_terminal) = match event {
                SseEvent::Token { text } => {
                    full_text.push_str(&text);
                    let update = TaskArtifactUpdateEvent {
                        task_id: task_for_bridge.clone(),
                        context_id: context_for_bridge.clone(),
                        artifact: Artifact {
                            artifact_id: "stream".into(),
                            name: None,
                            description: None,
                            parts: vec![Part::text(&text)],
                            metadata: None,
                        },
                        append: !full_text.is_empty(),
                        last_chunk: false,
                        metadata: None,
                        kind: "artifact-update".into(),
                    };
                    (JsonRpcResponse::success(
                        rpc_for_bridge.clone(),
                        serde_json::to_value(&update).unwrap_or_default(),
                    ), false)
                }
                SseEvent::ToolStart { name } => {
                    let update = TaskStatusUpdateEvent {
                        task_id: task_for_bridge.clone(),
                        context_id: context_for_bridge.clone(),
                        status: TaskStatus {
                            state: TaskState::Working,
                            message: Some(rune_a2a::types::Message {
                                message_id: Uuid::new_v4().to_string(),
                                role: Role::Agent,
                                parts: vec![Part::text(format!("Calling tool: {name}"))],
                                ..Default::default()
                            }),
                            timestamp: Some(chrono::Utc::now().to_rfc3339()),
                        },
                        metadata: None,
                        is_final: None,
                        kind: "status-update".into(),
                    };
                    (JsonRpcResponse::success(
                        rpc_for_bridge.clone(),
                        serde_json::to_value(&update).unwrap_or_default(),
                    ), false)
                }
                SseEvent::ToolDone { name, result } => {
                    let update = TaskArtifactUpdateEvent {
                        task_id: task_for_bridge.clone(),
                        context_id: context_for_bridge.clone(),
                        artifact: Artifact {
                            artifact_id: format!("tool-{name}"),
                            name: Some(name),
                            description: None,
                            parts: vec![Part::data(result)],
                            metadata: None,
                        },
                        append: false,
                        last_chunk: true,
                        metadata: None,
                        kind: "artifact-update".into(),
                    };
                    (JsonRpcResponse::success(
                        rpc_for_bridge.clone(),
                        serde_json::to_value(&update).unwrap_or_default(),
                    ), false)
                }
                SseEvent::Done { .. } => {
                    let update = TaskStatusUpdateEvent {
                        task_id: task_for_bridge.clone(),
                        context_id: context_for_bridge.clone(),
                        status: TaskStatus {
                            state: TaskState::Completed,
                            message: None,
                            timestamp: Some(chrono::Utc::now().to_rfc3339()),
                        },
                        metadata: None,
                        is_final: Some(true),
                        kind: "status-update".into(),
                    };
                    (JsonRpcResponse::success(
                        rpc_for_bridge.clone(),
                        serde_json::to_value(&update).unwrap_or_default(),
                    ), true)
                }
                SseEvent::Error { message } => {
                    let update = TaskStatusUpdateEvent {
                        task_id: task_for_bridge.clone(),
                        context_id: context_for_bridge.clone(),
                        status: TaskStatus {
                            state: TaskState::Failed,
                            message: Some(rune_a2a::types::Message {
                                message_id: Uuid::new_v4().to_string(),
                                role: Role::Agent,
                                parts: vec![Part::text(message)],
                                ..Default::default()
                            }),
                            timestamp: Some(chrono::Utc::now().to_rfc3339()),
                        },
                        metadata: None,
                        is_final: Some(true),
                        kind: "status-update".into(),
                    };
                    (JsonRpcResponse::success(
                        rpc_for_bridge.clone(),
                        serde_json::to_value(&update).unwrap_or_default(),
                    ), true)
                }
            };

            let payload = serde_json::to_value(&a2a_event).unwrap_or_default();
            if a2a_tx.send(payload).await.is_err() {
                tracing::debug!("SSE client disconnected, stopping bridge task");
                break;
            }
            if is_terminal {
                break;
            }
        }
    });

    let sse_stream = ReceiverStream::new(a2a_rx).map(|event| {
        let data = match serde_json::to_string(&event) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize SSE event");
                format!(r#"{{"error":"serialization failed: {}"}}"#, e)
            }
        };
        Ok::<Event, Infallible>(Event::default().data(data))
    });

    Ok(Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

// ---------------------------------------------------------------------------
// tasks/get
// ---------------------------------------------------------------------------

async fn handle_tasks_get(
    store: Arc<RuneStore>,
    rpc_req: JsonRpcRequest,
) -> Result<Response, GatewayError> {
    let rpc_id = rpc_req.id.clone();

    let params: GetTaskParams = serde_json::from_value(rpc_req.params)
        .map_err(|e| GatewayError::Internal(format!("invalid params: {e}")))?;

    let row = store
        .get_a2a_task(&params.id)
        .await
        .map_err(rune_runtime::RuntimeError::RuntimeStore)?
        .ok_or_else(|| GatewayError::Internal("task_not_found".into()))?;

    let response_summary = store
        .get_request_response_summary(&row.request_id)
        .await
        .map_err(rune_runtime::RuntimeError::RuntimeStore)?;

    let task_state = match row.state.as_str() {
        "submitted" => TaskState::Submitted,
        "working" | "running" => TaskState::Working,
        "completed" => TaskState::Completed,
        "failed" => TaskState::Failed,
        "canceled" => TaskState::Canceled,
        _ => TaskState::Working,
    };

    let artifacts = if task_state == TaskState::Completed {
        let text = response_summary
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
            .unwrap_or_default();

        Some(vec![Artifact {
            artifact_id: Uuid::new_v4().to_string(),
            name: Some("response".into()),
            description: None,
            parts: vec![Part::text(text)],
            metadata: None,
        }])
    } else {
        None
    };

    let history = if params.history_length != Some(0) {
        let limit = params.history_length.unwrap_or(50) as i64;
        load_a2a_history(&store, &row.session_id, limit).await?
    } else {
        None
    };

    let task = Task {
        id: row.task_id,
        context_id: Some(row.context_id),
        status: TaskStatus {
            state: task_state,
            message: None,
            timestamp: Some(row.updated_at),
        },
        artifacts,
        history,
        metadata: None,
        kind: "task".into(),
    };

    let result = serde_json::to_value(&task)
        .map_err(|e| GatewayError::Internal(e.to_string()))?;
    Ok(Json(JsonRpcResponse::success(rpc_id, result)).into_response())
}

// ---------------------------------------------------------------------------
// tasks/cancel
// ---------------------------------------------------------------------------

async fn handle_tasks_cancel(
    store: Arc<RuneStore>,
    rpc_req: JsonRpcRequest,
) -> Result<Response, GatewayError> {
    let rpc_id = rpc_req.id.clone();

    let params: CancelTaskParams = serde_json::from_value(rpc_req.params)
        .map_err(|e| GatewayError::Internal(format!("invalid params: {e}")))?;

    let rows_affected = store
        .cancel_a2a_task(&params.id)
        .await
        .map_err(rune_runtime::RuntimeError::RuntimeStore)?;

    if rows_affected == 0 {
        let resp = JsonRpcResponse::error(
            rpc_id,
            JsonRpcError {
                code: TASK_NOT_CANCELABLE,
                message: format!("task {} not found or already in terminal state", params.id),
                data: None,
            },
        );
        return Ok(Json(resp).into_response());
    }

    let task = Task {
        id: params.id,
        context_id: None,
        status: TaskStatus {
            state: TaskState::Canceled,
            message: None,
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        },
        artifacts: None,
        history: None,
        metadata: None,
        kind: "task".into(),
    };

    let result = serde_json::to_value(&task)
        .map_err(|e| GatewayError::Internal(e.to_string()))?;
    Ok(Json(JsonRpcResponse::success(rpc_id, result)).into_response())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn gateway_base_url(state: &crate::AppState) -> String {
    state.env.gateway_base_url.clone()
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
}

async fn resolve_deployment(store: &RuneStore, agent_name: &str) -> Result<Uuid, GatewayError> {
    store
        .resolve_deployment_for_agent(agent_name)
        .await
        .map_err(rune_runtime::RuntimeError::Storage)?
        .ok_or_else(|| GatewayError::NotFound(format!("no active deployment for agent '{agent_name}'")))
}

async fn resolve_or_create_session(
    sessions: &SessionManager,
    deployment_id: Uuid,
    context_id: Option<&str>,
) -> Result<(Uuid, String), GatewayError> {
    if let Some(ctx) = context_id {
        if let Ok(sid) = ctx.parse::<Uuid>() {
            if let Ok(Some(dep)) = sessions.get_deployment_id(sid).await {
                if dep == deployment_id {
                    return Ok((sid, ctx.to_string()));
                }
            }
        }
    }
    let sid = sessions.create(deployment_id, None, None).await?;
    Ok((sid, sid.to_string()))
}

fn parts_to_input(parts: &[Part]) -> serde_json::Value {
    let mut texts = Vec::new();
    let mut data_parts = Vec::new();

    for part in parts {
        match part {
            Part::Text { text, .. } => texts.push(text.as_str()),
            Part::Data { data, .. } => data_parts.push(data.clone()),
            Part::File { .. } => {}
        }
    }

    if !data_parts.is_empty() {
        if data_parts.len() == 1 {
            return data_parts.into_iter().next().unwrap();
        }
        return serde_json::Value::Array(data_parts);
    }

    serde_json::Value::String(texts.join("\n"))
}

fn extract_call_depth(message: &rune_a2a::types::Message) -> u32 {
    message
        .metadata
        .as_ref()
        .and_then(|m| m.get("rune_call_depth"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32
}

async fn load_a2a_history(
    store: &RuneStore,
    session_id: &str,
    limit: i64,
) -> Result<Option<Vec<rune_a2a::types::Message>>, GatewayError> {
    let rows = store
        .load_a2a_history(session_id, limit)
        .await
        .map_err(rune_runtime::RuntimeError::RuntimeStore)?;

    if rows.is_empty() {
        return Ok(None);
    }

    let messages: Vec<rune_a2a::types::Message> = rows
        .into_iter()
        .rev()
        .map(|r| {
            let role = if r.role == "assistant" || r.role == "agent" {
                Role::Agent
            } else {
                Role::User
            };
            let text = serde_json::from_str::<serde_json::Value>(&r.content)
                .ok()
                .and_then(|v| {
                    if let Some(s) = v.as_str() {
                        Some(s.to_string())
                    } else if v.is_array() {
                        v.as_array().and_then(|arr| {
                            arr.iter()
                                .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
                                .next()
                                .map(String::from)
                        })
                    } else {
                        Some(v.to_string())
                    }
                })
                .unwrap_or(r.content);

            rune_a2a::types::Message {
                message_id: Uuid::new_v4().to_string(),
                role,
                parts: vec![Part::text(text)],
                ..Default::default()
            }
        })
        .collect();

    Ok(Some(messages))
}
