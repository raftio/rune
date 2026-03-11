use serde::{Deserialize, Serialize};
use std::io::Cursor;
use uuid::Uuid;

use rune_store_runtime::SessionMessage;

use crate::models::{NewReplica, ReplicaState};
use crate::ops::registry::RegisterVersionInput;

openraft::declare_raft_types!(
    pub RuneTypeConfig:
        D            = WriteCommand,
        R            = WriteResponse,
        NodeId       = u64,
        Node         = openraft::BasicNode,
        Entry        = openraft::Entry<RuneTypeConfig>,
        SnapshotData = Cursor<Vec<u8>>,
);

/// Every write operation expressed as a serializable command for Raft log replication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WriteCommand {
    // -- Deployments --
    CreateDeployment(crate::ops::deployments::CreateDeploymentInput),
    SetDeploymentActive { id: Uuid },
    ScaleDeployment { id: Uuid, replicas: i32 },
    DeleteDeployment { id: Uuid },
    DeleteDeploymentForce { id: Uuid },

    // -- Replicas --
    InsertReplica(NewReplica),
    SetReplicaReady { id: Uuid },
    SetReplicaState { id: Uuid, state: ReplicaState },
    HeartbeatReplica { id: Uuid },
    MarkStaleReplicasFailed { threshold_secs: i64 },
    IncrementReplicaLoad { id: Uuid },
    DecrementReplicaLoad { id: Uuid },
    UpdateReplicaBackendInstanceId { id: Uuid, container_id: String },

    // -- Sessions --
    CreateSession {
        deployment_id: Uuid,
        tenant_id: Option<String>,
        routing_key: Option<String>,
    },
    CloseSession { id: Uuid },
    AppendSessionMessage {
        session_id: Uuid,
        role: String,
        content: serde_json::Value,
        step: i64,
    },
    AssignSessionReplica { session_id: Uuid, replica_id: Uuid },
    CheckpointSession {
        session_id: Uuid,
        messages: Vec<SessionMessage>,
        step: i64,
    },
    ApplySessionRestore {
        session_id: Uuid,
        messages: Vec<SessionMessage>,
    },

    // -- Requests --
    InsertRequest {
        request_id: Uuid,
        session_id: Uuid,
        deployment_id: Uuid,
        replica_id: Option<Uuid>,
        payload: serde_json::Value,
    },
    UpdateRequestCompleted {
        request_id: Uuid,
        output: serde_json::Value,
    },
    UpdateRequestStatus {
        request_id: Uuid,
        status: String,
    },

    // -- Tools --
    InsertToolInvocation {
        invocation_id: String,
        request_id: Uuid,
        tool_name: String,
        tool_runtime: String,
        input_payload: String,
    },
    UpdateToolInvocationCompleted {
        invocation_id: String,
        output_payload: String,
    },
    UpdateToolInvocationFailed {
        invocation_id: String,
        error_message: String,
    },
    InsertAgentCall {
        call_id: String,
        caller_task_id: String,
        caller_agent: String,
        callee_agent: String,
        callee_endpoint: String,
        depth: i64,
    },
    UpdateAgentCallCalleeTask {
        call_id: String,
        callee_task_id: String,
    },
    UpdateAgentCallCompleted {
        call_id: String,
        latency_ms: i64,
    },

    // -- A2A --
    InsertA2ATask {
        task_id: String,
        context_id: String,
        request_id: String,
        session_id: String,
        agent_name: String,
    },
    UpdateA2ATaskState { task_id: String, state: String },
    CancelA2ATask { task_id: String },

    // -- Cache --
    SetSessionRoute {
        session_id: Uuid,
        replica_id: Uuid,
        ttl_secs: Option<i64>,
    },
    RateLimitCheck {
        key: String,
        window_seconds: i64,
        max_count: i64,
    },
    CleanupExpiredCache,

    // -- Registry --
    RegisterAgentVersion(RegisterVersionInput),

    // -- Policy --
    AuditPolicyDecision {
        kind: String,
        resource: String,
        allowed: bool,
        reason: Option<String>,
        request_id: Option<String>,
    },

    // -- Schedules --
    UpdateScheduleLastRun { schedule_id: String, now: String },
    UpdateScheduleNextRun { schedule_id: String, next_run: String },

    // -- Agent Ops --
    SpawnAgent { agent_name: String },
    KillAgent { agent_id: String },
}

/// Response from applying a WriteCommand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WriteResponse {
    Ok,
    Id(Uuid),
    StringId(String),
    IdPair(String, String),
    Count(u64),
    RateLimitResult(bool),
}
