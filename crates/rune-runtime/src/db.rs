/// Thin re-exports delegating to `rune_storage`.
/// Kept for backward compatibility during transition.
pub use rune_storage::models::NewReplica;
pub use rune_storage::models::AgentVersionImage;

use rune_storage::{RuneStore, RuntimeStore};
use uuid::Uuid;

use crate::models::{Deployment, ReplicaState};

pub async fn get_schedulable_deployments(store: &RuneStore) -> Result<Vec<Deployment>, rune_storage::StorageError> {
    store.get_schedulable_deployments().await
}

pub async fn get_agent_version_image(store: &RuneStore, id: Uuid) -> Result<Option<AgentVersionImage>, rune_storage::StorageError> {
    store.get_agent_version_image(id).await
}

pub async fn set_deployment_active(store: &RuneStore, id: Uuid) -> Result<(), rune_storage::StorageError> {
    store.set_deployment_active(id).await
}

pub async fn count_healthy_replicas(store: &RuneStore, deployment_id: Uuid) -> Result<i64, rune_storage::StorageError> {
    store.count_healthy_replicas(deployment_id).await
}

pub async fn insert_replica(store: &RuneStore, r: &NewReplica) -> Result<Uuid, rune_storage::StorageError> {
    store.insert_replica(r).await
}

pub async fn set_replica_ready(store: &RuneStore, id: Uuid) -> Result<(), rune_storage::StorageError> {
    store.set_replica_ready(id).await
}

pub async fn set_replica_state(store: &RuneStore, id: Uuid, state: ReplicaState) -> Result<(), rune_storage::StorageError> {
    store.set_replica_state(id, state).await
}

pub async fn heartbeat_replica(store: &RuneStore, id: Uuid) -> Result<(), rune_storage::StorageError> {
    store.heartbeat_replica(id).await
}

pub async fn get_drainable_replica_ids(store: &RuneStore, deployment_id: Uuid, limit: i64) -> Result<Vec<Uuid>, rune_storage::StorageError> {
    store.get_drainable_replica_ids(deployment_id, limit).await
}

pub async fn mark_stale_replicas_failed(store: &RuneStore, threshold_secs: i64) -> Result<u64, rune_storage::StorageError> {
    store.mark_stale_replicas_failed(threshold_secs).await
}

pub async fn select_least_loaded_replica(store: &RuneStore, deployment_id: Uuid) -> Result<Option<Uuid>, rune_storage::StorageError> {
    store.select_least_loaded_replica(deployment_id).await
}

pub async fn is_replica_available(store: &RuneStore, replica_id: Uuid) -> Result<bool, rune_storage::StorageError> {
    store.is_replica_available(replica_id).await
}

pub async fn increment_replica_load(store: &RuneStore, replica_id: Uuid) -> Result<(), rune_storage::StorageError> {
    store.increment_replica_load(replica_id).await
}

pub async fn decrement_replica_load(store: &RuneStore, replica_id: Uuid) -> Result<(), rune_storage::StorageError> {
    store.decrement_replica_load(replica_id).await
}

pub async fn get_session_assigned_replica(store: &RuneStore, session_id: Uuid) -> Result<Option<Uuid>, rune_storage::RuntimeStoreError> {
    store.get_session_assigned_replica(session_id).await
}

pub async fn assign_session_replica(store: &RuneStore, session_id: Uuid, replica_id: Uuid) -> Result<(), rune_storage::RuntimeStoreError> {
    store.assign_session_replica(session_id, replica_id).await
}
