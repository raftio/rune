use std::convert::Infallible;

use axum::{
    extract::{Path, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt as _;
use uuid::Uuid;

use crate::error::GatewayError;
use rune_runtime::{LlmClient, Planner, ReplicaRouter, SessionManager, SseEvent, StubPlanner, ToolDispatcher};
use rune_storage::RuntimeStore;

#[derive(Debug, Deserialize)]
pub struct InvokeRequest {
    pub session_id: Option<Uuid>,
    pub input: serde_json::Value,
    #[serde(default)]
    pub stream: bool,
}

#[derive(Debug, Serialize)]
pub struct InvokeResponse {
    pub request_id: Uuid,
    pub session_id: Uuid,
    pub output: serde_json::Value,
}

pub async fn invoke(
    State(state): State<crate::AppState>,
    Path(agent_name): Path<String>,
    Json(req): Json<InvokeRequest>,
) -> Result<Response, GatewayError> {
    let start = std::time::Instant::now();

    let deployment_id: Uuid = state.store
        .resolve_deployment_for_agent(&agent_name)
        .await
        .map_err(rune_runtime::RuntimeError::Storage)?
        .ok_or_else(|| {
            rune_runtime::metrics::record_request_duration(&agent_name, "error", start.elapsed().as_secs_f64());
            GatewayError::NotFound(format!("no active deployment for agent '{agent_name}'"))
        })?;

    let sessions = SessionManager::new(state.store.clone());
    let session_id = match req.session_id {
        Some(sid) => {
            let dep = sessions.get_deployment_id(sid).await?;
            if dep != Some(deployment_id) {
                return Err(GatewayError::NotFound(format!(
                    "session {sid} not found for this deployment"
                )));
            }
            sid
        }
        None => sessions.create(deployment_id, None, None).await?,
    };

    let router = ReplicaRouter::new(state.store.clone());
    let lease = router.acquire(deployment_id, session_id).await
        .map_err(|_| {
            rune_runtime::metrics::record_request_duration(&agent_name, "error", start.elapsed().as_secs_f64());
            GatewayError::NoReplicaAvailable(deployment_id.to_string())
        })?;
    let replica_id = lease.replica_id;

    let env = &state.env;
    let plan = crate::routes::resolve_plan(&agent_name, env.agent_packages_dir.as_deref());
    let tool_ctx = crate::routes::shared_tool_context(&state.store, env);
    let policy = rune_runtime::PolicyEngine::new(
        plan.toolset.clone().into_iter(),
        plan.models.clone(),
    );
    let tools = ToolDispatcher::new(plan.tools.clone(), plan.agent_dir.clone())
        .with_tool_context(tool_ctx)
        .with_policy(policy)
        .with_caller_networks(plan.networks.clone());

    let request_id = Uuid::new_v4();
    state.store
        .insert_request(request_id, session_id, deployment_id, Some(replica_id), &req.input)
        .await
        .map_err(rune_runtime::RuntimeError::RuntimeStore)?;

    if req.stream {
        let client = LlmClient::from_platform_env(env)
            .or_else(LlmClient::from_env)
            .ok_or_else(|| GatewayError::Internal(
                "streaming requires ANTHROPIC_API_KEY or OPENAI_API_KEY".into(),
            ))?;

        let (tx, rx) = mpsc::channel::<SseEvent>(64);
        let store2 = state.store.clone();

        tokio::spawn(async move {
            let planner = Planner::new(plan);
            let result = planner
                .run_stream(&client, &sessions, session_id, req.input, &tools, request_id, &store2, tx.clone())
                .await;

            let status = if result.is_ok() { "completed" } else { "failed" };
            if let Err(e) = store2.update_request_status(request_id, status).await {
                tracing::error!(
                    request_id = %request_id,
                    error = %e,
                    "failed to persist request status after stream — request may show as running"
                );
            }

            if let Err(ref e) = result {
                tracing::error!(request_id = %request_id, error = %e, "stream planner error");
                let _ = tx.send(SseEvent::Error { message: e.to_string() }).await;
            }

            lease.release().await;
        });

        let sse_stream = ReceiverStream::new(rx).map(|event| {
            let data = serde_json::to_string(&event).unwrap_or_default();
            Ok::<Event, Infallible>(Event::default().data(data))
        });

        rune_runtime::metrics::record_request_duration(&agent_name, "ok", start.elapsed().as_secs_f64());
        return Ok(Sse::new(sse_stream)
            .keep_alive(KeepAlive::default())
            .into_response());
    }

    let output = if let Some(client) = LlmClient::from_platform_env(env).or_else(LlmClient::from_env) {
        let planner = Planner::new(plan);
        planner
            .run(&client, &sessions, session_id, req.input, &tools, request_id, &state.store)
            .await?
    } else {
        let stub = StubPlanner::new(&agent_name);
        stub.run(&sessions, session_id, req.input).await?
    };

    lease.release().await;

    state.store
        .update_request_completed(request_id, &output)
        .await
        .map_err(rune_runtime::RuntimeError::RuntimeStore)?;

    rune_runtime::metrics::record_request_duration(&agent_name, "ok", start.elapsed().as_secs_f64());
    Ok(Json(InvokeResponse { request_id, session_id, output }).into_response())
}

