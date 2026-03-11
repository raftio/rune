use async_trait::async_trait;
use uuid::Uuid;

use crate::error::RuntimeError;
use crate::models::{Deployment, HealthStatus, Replica};

/// The Agent Runtime Contract.
/// Every execution backend (WASM, Docker, Kubernetes) must implement this trait.
#[async_trait]
pub trait RuntimeBackend: Send + Sync {
    /// Start a new replica for the given deployment.
    async fn start_replica(&self, deployment: &Deployment) -> Result<Replica, RuntimeError>;

    /// Gracefully drain a replica — stop accepting new requests, wait for active ones.
    async fn drain_replica(&self, replica_id: Uuid) -> Result<(), RuntimeError>;

    /// Forcefully stop a replica.
    async fn stop_replica(&self, replica_id: Uuid) -> Result<(), RuntimeError>;

    /// Check health of a running replica.
    async fn health(&self, replica_id: Uuid) -> Result<HealthStatus, RuntimeError>;

    /// Fetch runtime stats (load, memory, etc.) for a replica.
    async fn stats(&self, replica_id: Uuid) -> Result<ReplicaStats, RuntimeError>;
}

#[derive(Debug, Clone)]
pub struct ReplicaStats {
    pub current_load: u32,
    pub memory_bytes: Option<u64>,
}
