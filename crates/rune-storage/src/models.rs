use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Re-export runtime types so existing callers keep working.
pub use rune_store_runtime::{A2aTaskRow, CheckpointBlob, HistoryMessage, SessionMessage};

// ---------------------------------------------------------------------------
// Deployment
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub id: Uuid,
    pub agent_version_id: Uuid,
    pub namespace: String,
    pub rollout_alias: String,
    pub desired_replicas: i32,
    pub min_replicas: i32,
    pub max_replicas: i32,
    pub concurrency_limit: i32,
    pub config_ref: Option<String>,
    pub secret_ref: Option<String>,
    pub env_json: Option<String>,
    pub status: DeploymentStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentStatus {
    Pending,
    Active,
    Draining,
    Stopped,
}

impl std::fmt::Display for DeploymentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Active => write!(f, "active"),
            Self::Draining => write!(f, "draining"),
            Self::Stopped => write!(f, "stopped"),
        }
    }
}

// ---------------------------------------------------------------------------
// Replica
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replica {
    pub id: Uuid,
    pub deployment_id: Uuid,
    pub backend_type: BackendType,
    pub backend_instance_id: String,
    pub node_name: Option<String>,
    pub endpoint: Option<String>,
    pub state: ReplicaState,
    pub concurrency_limit: i32,
    pub current_load: i32,
    pub started_at: Option<DateTime<Utc>>,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReplicaState {
    Starting,
    Ready,
    Draining,
    Stopped,
    Failed,
}

impl std::fmt::Display for ReplicaState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Starting => write!(f, "starting"),
            Self::Ready => write!(f, "ready"),
            Self::Draining => write!(f, "draining"),
            Self::Stopped => write!(f, "stopped"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendType {
    Wasm,
    Docker,
    Kubernetes,
}

impl std::fmt::Display for BackendType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wasm => write!(f, "wasm"),
            Self::Docker => write!(f, "docker"),
            Self::Kubernetes => write!(f, "kubernetes"),
        }
    }
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub healthy: bool,
    pub message: Option<String>,
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub deployment_id: Uuid,
    pub tenant_id: Option<String>,
    pub routing_key: Option<String>,
    pub assigned_replica_id: Option<Uuid>,
    pub state_ref: Option<String>,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Idle,
    Closed,
    Failed,
}

// ---------------------------------------------------------------------------
// Input types for write operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewReplica {
    pub deployment_id: Uuid,
    pub backend_type: BackendType,
    pub backend_instance_id: String,
    pub concurrency_limit: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentVersionImage {
    pub image_ref: String,
    pub image_digest: String,
    pub signature_ref: Option<String>,
}

