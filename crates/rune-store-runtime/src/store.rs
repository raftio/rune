use async_trait::async_trait;
use uuid::Uuid;

use crate::error::RuntimeStoreError;
use crate::types::{A2aTaskRow, HistoryMessage, SessionMessage};

/// Abstraction over the storage layer for runtime-generated data:
/// sessions, requests, A2A tasks, and tool invocations.
///
/// This data is created by end-user interaction and agent execution —
/// distinct from infrastructure state (deployments, replicas) and
/// configuration (policy, env).
#[async_trait]
pub trait RuntimeStore: Send + Sync {
    // -----------------------------------------------------------------------
    // Sessions
    // -----------------------------------------------------------------------

    async fn create_session(
        &self,
        deployment_id: Uuid,
        tenant_id: Option<&str>,
        routing_key: Option<&str>,
    ) -> Result<Uuid, RuntimeStoreError>;

    async fn load_session_messages(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionMessage>, RuntimeStoreError>;

    async fn append_session_message(
        &self,
        session_id: Uuid,
        role: &str,
        content: &serde_json::Value,
        step: i64,
    ) -> Result<(), RuntimeStoreError>;

    async fn close_session(&self, session_id: Uuid) -> Result<(), RuntimeStoreError>;

    async fn get_session_deployment_id(
        &self,
        session_id: Uuid,
    ) -> Result<Option<Uuid>, RuntimeStoreError>;

    async fn get_session_assigned_replica(
        &self,
        session_id: Uuid,
    ) -> Result<Option<Uuid>, RuntimeStoreError>;

    async fn assign_session_replica(
        &self,
        session_id: Uuid,
        replica_id: Uuid,
    ) -> Result<(), RuntimeStoreError>;

    async fn checkpoint_session(
        &self,
        session_id: Uuid,
        messages: &[SessionMessage],
        step: i64,
    ) -> Result<(), RuntimeStoreError>;

    async fn restore_session(
        &self,
        session_id: Uuid,
    ) -> Result<Option<(Vec<SessionMessage>, i64)>, RuntimeStoreError>;

    async fn apply_session_restore(
        &self,
        session_id: Uuid,
        messages: &[SessionMessage],
    ) -> Result<(), RuntimeStoreError>;

    async fn get_session_info(
        &self,
        session_id: Uuid,
    ) -> Result<Option<serde_json::Value>, RuntimeStoreError>;

    // -----------------------------------------------------------------------
    // Session routing (runtime cache: where is this session routed?)
    // -----------------------------------------------------------------------

    async fn get_session_route(
        &self,
        session_id: Uuid,
    ) -> Result<Option<Uuid>, RuntimeStoreError>;

    async fn set_session_route(
        &self,
        session_id: Uuid,
        replica_id: Uuid,
        ttl_secs: Option<i64>,
    ) -> Result<(), RuntimeStoreError>;

    // -----------------------------------------------------------------------
    // Requests
    // -----------------------------------------------------------------------

    async fn insert_request(
        &self,
        request_id: Uuid,
        session_id: Uuid,
        deployment_id: Uuid,
        replica_id: Option<Uuid>,
        payload: &serde_json::Value,
    ) -> Result<(), RuntimeStoreError>;

    async fn update_request_completed(
        &self,
        request_id: Uuid,
        output: &serde_json::Value,
    ) -> Result<(), RuntimeStoreError>;

    async fn update_request_status(
        &self,
        request_id: Uuid,
        status: &str,
    ) -> Result<(), RuntimeStoreError>;

    async fn get_request_response_summary(
        &self,
        request_id: &str,
    ) -> Result<Option<String>, RuntimeStoreError>;

    // -----------------------------------------------------------------------
    // A2A tasks
    // -----------------------------------------------------------------------

    async fn insert_a2a_task(
        &self,
        task_id: &str,
        context_id: &str,
        request_id: &str,
        session_id: &str,
        agent_name: &str,
    ) -> Result<(), RuntimeStoreError>;

    async fn update_a2a_task_state(
        &self,
        task_id: &str,
        state: &str,
    ) -> Result<(), RuntimeStoreError>;

    async fn get_a2a_task(
        &self,
        task_id: &str,
    ) -> Result<Option<A2aTaskRow>, RuntimeStoreError>;

    async fn cancel_a2a_task(&self, task_id: &str) -> Result<u64, RuntimeStoreError>;

    async fn load_a2a_history(
        &self,
        session_id: &str,
        limit: i64,
    ) -> Result<Vec<HistoryMessage>, RuntimeStoreError>;

    // -----------------------------------------------------------------------
    // Tool invocations
    // -----------------------------------------------------------------------

    async fn insert_tool_invocation(
        &self,
        invocation_id: &str,
        request_id: Uuid,
        tool_name: &str,
        tool_runtime: &str,
        input_payload: &str,
    ) -> Result<(), RuntimeStoreError>;

    async fn update_tool_invocation_completed(
        &self,
        invocation_id: &str,
        output_payload: &str,
    ) -> Result<(), RuntimeStoreError>;

    async fn update_tool_invocation_failed(
        &self,
        invocation_id: &str,
        error_message: &str,
    ) -> Result<(), RuntimeStoreError>;

    async fn insert_agent_call(
        &self,
        call_id: &str,
        caller_task_id: &str,
        caller_agent: &str,
        callee_agent: &str,
        callee_endpoint: &str,
        depth: i64,
    ) -> Result<(), RuntimeStoreError>;

    async fn update_agent_call_callee_task(
        &self,
        call_id: &str,
        callee_task_id: &str,
    ) -> Result<(), RuntimeStoreError>;

    async fn update_agent_call_completed(
        &self,
        call_id: &str,
        latency_ms: i64,
    ) -> Result<(), RuntimeStoreError>;
}
