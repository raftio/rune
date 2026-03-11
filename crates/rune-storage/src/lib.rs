pub mod error;
pub mod migrations;
pub mod models;
pub mod object_store;
pub mod ops;
pub mod pool;
pub mod raft;

pub use error::StorageError;
pub use models::*;
pub use pool::PoolConfig;
pub use raft::{RuneTypeConfig, WriteCommand, WriteResponse};

// Re-export RuntimeStore and its error so callers only need rune-storage.
pub use rune_store_runtime::{RuntimeStore, RuntimeStoreError};

use async_trait::async_trait;
use sqlx::SqlitePool;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Error conversion: StorageError → RuntimeStoreError
// ---------------------------------------------------------------------------

impl From<StorageError> for RuntimeStoreError {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::NotFound(msg) => RuntimeStoreError::NotFound(msg),
            StorageError::Serialization(msg) => RuntimeStoreError::Serialization(msg),
            other => RuntimeStoreError::Backend(other.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Store mode
// ---------------------------------------------------------------------------

/// Controls whether writes go directly to SQLite or through Raft consensus.
pub enum StoreMode {
    /// Single-node — all writes go straight to the local database.
    Standalone,
    /// Multi-node cluster — writes are proposed to the Raft leader and
    /// replicated to all nodes before being applied.
    Cluster {
        raft: openraft::Raft<RuneTypeConfig>,
    },
}

impl Clone for StoreMode {
    fn clone(&self) -> Self {
        match self {
            Self::Standalone => Self::Standalone,
            Self::Cluster { raft } => Self::Cluster { raft: raft.clone() },
        }
    }
}

// ---------------------------------------------------------------------------
// RuneStore
// ---------------------------------------------------------------------------

/// Unified storage layer for all Rune state.
///
/// In **standalone** mode (default), all reads and writes go directly to the
/// local SQLite database.
///
/// In **cluster** mode, reads still go directly to local SQLite (eventually
/// consistent within Raft commit latency), while writes are proposed through
/// the Raft consensus protocol and applied to every node's SQLite once
/// committed.
#[derive(Clone)]
pub struct RuneStore {
    db: SqlitePool,
    mode: StoreMode,
}

impl RuneStore {
    /// Create a new standalone store.
    pub async fn new(config: PoolConfig) -> Result<Self, StorageError> {
        let db = pool::create_pool(&config).await?;
        Ok(Self {
            db,
            mode: StoreMode::Standalone,
        })
    }

    /// Create a store from an existing pool (standalone mode).
    pub fn from_pool(db: SqlitePool) -> Self {
        Self {
            db,
            mode: StoreMode::Standalone,
        }
    }

    /// Upgrade this store to cluster mode with the given Raft handle.
    pub fn enable_cluster(&mut self, raft: openraft::Raft<RuneTypeConfig>) {
        self.mode = StoreMode::Cluster { raft };
    }

    /// Whether this store is running in cluster (Raft) mode.
    pub fn is_cluster(&self) -> bool {
        matches!(self.mode, StoreMode::Cluster { .. })
    }

    pub async fn run_migrations(&self) -> Result<(), StorageError> {
        migrations::run_migrations(&self.db).await
    }

    /// Access the raw SqlitePool for backward compatibility during transition.
    pub fn pool(&self) -> &SqlitePool {
        &self.db
    }

    /// Check whether this node is the Raft leader. Always true in standalone.
    pub fn is_leader(&self) -> bool {
        match &self.mode {
            StoreMode::Standalone => true,
            StoreMode::Cluster { raft } => {
                let metrics = raft.metrics().borrow().clone();
                metrics.current_leader == Some(metrics.id)
            }
        }
    }

    /// Wait until this node becomes Raft leader. Returns immediately in standalone.
    pub async fn wait_for_leadership(&self) {
        if let StoreMode::Cluster { raft } = &self.mode {
            let mut rx = raft.metrics();
            loop {
                let m = rx.borrow().clone();
                if m.current_leader == Some(m.id) {
                    return;
                }
                if rx.changed().await.is_err() {
                    return;
                }
            }
        }
    }

    /// Wait until this node loses leadership.
    /// In standalone mode leadership is never lost, so this pends forever.
    pub async fn wait_for_leadership_lost(&self) {
        match &self.mode {
            StoreMode::Standalone => std::future::pending::<()>().await,
            StoreMode::Cluster { raft } => {
                let mut rx = raft.metrics();
                loop {
                    let m = rx.borrow().clone();
                    if m.current_leader != Some(m.id) {
                        return;
                    }
                    if rx.changed().await.is_err() {
                        return;
                    }
                }
            }
        }
    }

    /// Ensure linearizable read (wait for Raft commit to catch up).
    pub async fn ensure_linearizable(&self) -> Result<(), StorageError> {
        if let StoreMode::Cluster { raft } = &self.mode {
            raft.ensure_linearizable()
                .await
                .map_err(|e| StorageError::Raft(e.to_string()))?;
        }
        Ok(())
    }

    // =======================================================================
    // Internal: route writes through Raft or direct SQL
    // =======================================================================

    async fn propose(&self, cmd: WriteCommand) -> Result<WriteResponse, StorageError> {
        match &self.mode {
            StoreMode::Standalone => raft::store::apply_command(&self.db, cmd).await,
            StoreMode::Cluster { raft } => {
                let resp = raft
                    .client_write(cmd)
                    .await
                    .map_err(|e| StorageError::Raft(e.to_string()))?;
                Ok(resp.data)
            }
        }
    }

    fn expect_ok(resp: WriteResponse) -> Result<(), StorageError> {
        match resp {
            WriteResponse::Ok => Ok(()),
            _ => Err(StorageError::UnexpectedResponse),
        }
    }

    fn expect_id(resp: WriteResponse) -> Result<Uuid, StorageError> {
        match resp {
            WriteResponse::Id(id) => Ok(id),
            _ => Err(StorageError::UnexpectedResponse),
        }
    }

    fn expect_count(resp: WriteResponse) -> Result<u64, StorageError> {
        match resp {
            WriteResponse::Count(n) => Ok(n),
            _ => Err(StorageError::UnexpectedResponse),
        }
    }

    fn expect_rate_limit(resp: WriteResponse) -> Result<bool, StorageError> {
        match resp {
            WriteResponse::RateLimitResult(ok) => Ok(ok),
            _ => Err(StorageError::UnexpectedResponse),
        }
    }

    fn expect_string_id(resp: WriteResponse) -> Result<String, StorageError> {
        match resp {
            WriteResponse::StringId(id) => Ok(id),
            _ => Err(StorageError::UnexpectedResponse),
        }
    }

    fn expect_id_pair(resp: WriteResponse) -> Result<(String, String), StorageError> {
        match resp {
            WriteResponse::IdPair(a, b) => Ok((a, b)),
            _ => Err(StorageError::UnexpectedResponse),
        }
    }

    // =======================================================================
    // Deployments
    // =======================================================================

    /// Read: direct local SQL
    pub async fn get_schedulable_deployments(&self) -> Result<Vec<Deployment>, StorageError> {
        ops::deployments::get_schedulable(&self.db).await
    }

    /// Write: create or upsert deployment via Raft.
    pub async fn create_deployment(
        &self,
        input: ops::deployments::CreateDeploymentInput,
    ) -> Result<Uuid, StorageError> {
        let resp = self
            .propose(WriteCommand::CreateDeployment(input))
            .await?;
        Self::expect_id(resp)
    }

    /// Write
    pub async fn set_deployment_active(&self, id: Uuid) -> Result<(), StorageError> {
        let resp = self.propose(WriteCommand::SetDeploymentActive { id }).await?;
        Self::expect_ok(resp)
    }

    /// Read
    pub async fn get_agent_and_env_for_deployment(
        &self,
        deployment_id: Uuid,
    ) -> Result<Option<(String, Option<String>)>, StorageError> {
        ops::deployments::get_agent_and_env_for_deployment(&self.db, deployment_id).await
    }

    /// Read
    pub async fn resolve_deployment_for_agent(
        &self,
        agent_name: &str,
    ) -> Result<Option<Uuid>, StorageError> {
        ops::deployments::resolve_for_agent(&self.db, agent_name).await
    }

    /// Write
    pub async fn scale_deployment(
        &self,
        id: Uuid,
        desired_replicas: i32,
    ) -> Result<u64, StorageError> {
        let resp = self
            .propose(WriteCommand::ScaleDeployment {
                id,
                replicas: desired_replicas,
            })
            .await?;
        Self::expect_count(resp)
    }

    /// Write
    pub async fn delete_deployment(&self, id: Uuid) -> Result<u64, StorageError> {
        let resp = self.propose(WriteCommand::DeleteDeployment { id }).await?;
        Self::expect_count(resp)
    }

    /// Write: force-delete deployment (disables FK checks; leaves orphan rows).
    pub async fn delete_deployment_force(&self, id: Uuid) -> Result<u64, StorageError> {
        let resp = self
            .propose(WriteCommand::DeleteDeploymentForce { id })
            .await?;
        Self::expect_count(resp)
    }

    /// Read
    pub async fn list_deployed_agent_names(&self) -> Result<Vec<String>, StorageError> {
        ops::deployments::list_deployed_agent_names(&self.db).await
    }

    /// Read
    pub async fn verify_agent_deployed(&self, agent_name: &str) -> Result<bool, StorageError> {
        ops::deployments::verify_agent_deployed(&self.db, agent_name).await
    }

    // =======================================================================
    // Replicas
    // =======================================================================

    /// Read
    pub async fn count_healthy_replicas(
        &self,
        deployment_id: Uuid,
    ) -> Result<i64, StorageError> {
        ops::replicas::count_healthy(&self.db, deployment_id).await
    }

    /// Write
    pub async fn insert_replica(&self, r: &NewReplica) -> Result<Uuid, StorageError> {
        let resp = self
            .propose(WriteCommand::InsertReplica(r.clone()))
            .await?;
        Self::expect_id(resp)
    }

    /// Write
    pub async fn set_replica_ready(&self, id: Uuid) -> Result<(), StorageError> {
        let resp = self.propose(WriteCommand::SetReplicaReady { id }).await?;
        Self::expect_ok(resp)
    }

    /// Write
    pub async fn set_replica_state(
        &self,
        id: Uuid,
        state: ReplicaState,
    ) -> Result<(), StorageError> {
        let resp = self
            .propose(WriteCommand::SetReplicaState { id, state })
            .await?;
        Self::expect_ok(resp)
    }

    /// Write
    pub async fn heartbeat_replica(&self, id: Uuid) -> Result<(), StorageError> {
        let resp = self.propose(WriteCommand::HeartbeatReplica { id }).await?;
        Self::expect_ok(resp)
    }

    /// Read
    pub async fn get_drainable_replica_ids(
        &self,
        deployment_id: Uuid,
        limit: i64,
    ) -> Result<Vec<Uuid>, StorageError> {
        ops::replicas::get_drainable_ids(&self.db, deployment_id, limit).await
    }

    /// Write
    pub async fn mark_stale_replicas_failed(
        &self,
        threshold_secs: i64,
    ) -> Result<u64, StorageError> {
        let resp = self
            .propose(WriteCommand::MarkStaleReplicasFailed { threshold_secs })
            .await?;
        Self::expect_count(resp)
    }

    /// Read
    pub async fn select_least_loaded_replica(
        &self,
        deployment_id: Uuid,
    ) -> Result<Option<Uuid>, StorageError> {
        ops::replicas::select_least_loaded(&self.db, deployment_id).await
    }

    /// Read
    pub async fn is_replica_available(&self, replica_id: Uuid) -> Result<bool, StorageError> {
        ops::replicas::is_available(&self.db, replica_id).await
    }

    /// Write
    pub async fn increment_replica_load(&self, replica_id: Uuid) -> Result<(), StorageError> {
        let resp = self
            .propose(WriteCommand::IncrementReplicaLoad { id: replica_id })
            .await?;
        Self::expect_ok(resp)
    }

    /// Write
    pub async fn decrement_replica_load(&self, replica_id: Uuid) -> Result<(), StorageError> {
        let resp = self
            .propose(WriteCommand::DecrementReplicaLoad { id: replica_id })
            .await?;
        Self::expect_ok(resp)
    }

    /// Read
    pub async fn get_replica_current_load(&self, replica_id: Uuid) -> Result<i64, StorageError> {
        ops::replicas::get_current_load(&self.db, replica_id).await
    }

    /// Read
    pub async fn get_replica_backend_instance_id(
        &self,
        replica_id: Uuid,
    ) -> Result<Option<String>, StorageError> {
        ops::replicas::get_backend_instance_id(&self.db, replica_id).await
    }

    /// Write
    pub async fn update_replica_backend_instance_id(
        &self,
        replica_id: Uuid,
        container_id: &str,
    ) -> Result<(), StorageError> {
        let resp = self
            .propose(WriteCommand::UpdateReplicaBackendInstanceId {
                id: replica_id,
                container_id: container_id.to_string(),
            })
            .await?;
        Self::expect_ok(resp)
    }

    // =======================================================================
    // Cache (infra: rate limiting, replica load)
    // =======================================================================

    /// Write (read+write — goes through consensus to ensure consistent counts)
    pub async fn rate_limit_check(
        &self,
        key: &str,
        window_seconds: i64,
        max_count: i64,
    ) -> Result<bool, StorageError> {
        let resp = self
            .propose(WriteCommand::RateLimitCheck {
                key: key.to_string(),
                window_seconds,
                max_count,
            })
            .await?;
        Self::expect_rate_limit(resp)
    }

    /// Write
    pub async fn cleanup_expired_cache(&self) -> Result<u64, StorageError> {
        let resp = self.propose(WriteCommand::CleanupExpiredCache).await?;
        Self::expect_count(resp)
    }

    // =======================================================================
    // Registry
    // =======================================================================

    /// Write
    pub async fn register_agent_version(
        &self,
        input: &ops::registry::RegisterVersionInput,
    ) -> Result<ops::registry::AgentVersionRow, StorageError> {
        let resp = self
            .propose(WriteCommand::RegisterAgentVersion(input.clone()))
            .await?;
        let id = Self::expect_string_id(resp)?;
        let rows = ops::registry::list(&self.db).await?;
        rows.into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| StorageError::NotFound("registered version not found".into()))
    }

    /// Read
    pub async fn list_agent_versions(
        &self,
    ) -> Result<Vec<ops::registry::AgentVersionRow>, StorageError> {
        ops::registry::list(&self.db).await
    }

    /// Read
    pub async fn get_agent_version_image(
        &self,
        agent_version_id: Uuid,
    ) -> Result<Option<AgentVersionImage>, StorageError> {
        ops::registry::get_version_image(&self.db, agent_version_id).await
    }

    /// Read
    pub async fn check_deployment_admission(
        &self,
        agent_version_id: Uuid,
    ) -> Result<(), StorageError> {
        ops::registry::check_admission(&self.db, agent_version_id).await
    }

    /// Read
    pub async fn agent_version_exists(
        &self,
        agent_version_id: Uuid,
    ) -> Result<bool, StorageError> {
        ops::registry::version_exists(&self.db, agent_version_id).await
    }

    // =======================================================================
    // Network
    // =======================================================================

    /// Read
    pub async fn get_agent_networks(
        &self,
        agent_name: &str,
    ) -> Result<Vec<String>, StorageError> {
        ops::network::get_agent_networks(&self.db, agent_name).await
    }

    /// Read
    pub async fn get_agent_direct_replica_endpoint(
        &self,
        agent_name: &str,
    ) -> Result<Option<String>, StorageError> {
        ops::network::get_direct_replica_endpoint(&self.db, agent_name).await
    }

    // =======================================================================
    // Policy
    // =======================================================================

    /// Write
    pub async fn audit_policy_decision(
        &self,
        kind: &str,
        resource: &str,
        allowed: bool,
        reason: Option<&str>,
        request_id: Option<&str>,
    ) -> Result<(), StorageError> {
        let resp = self
            .propose(WriteCommand::AuditPolicyDecision {
                kind: kind.to_string(),
                resource: resource.to_string(),
                allowed,
                reason: reason.map(String::from),
                request_id: request_id.map(String::from),
            })
            .await?;
        Self::expect_ok(resp)
    }

    // =======================================================================
    // Schedules
    // =======================================================================

    /// Read
    pub async fn list_enabled_schedules(
        &self,
    ) -> Result<Vec<ops::schedules::ScheduleRow>, StorageError> {
        ops::schedules::list_enabled(&self.db).await
    }

    /// Read
    pub async fn get_schedule_last_run(
        &self,
        schedule_id: &str,
    ) -> Result<Option<String>, StorageError> {
        ops::schedules::get_last_run(&self.db, schedule_id).await
    }

    /// Write
    pub async fn update_schedule_last_run(
        &self,
        schedule_id: &str,
        now: &str,
    ) -> Result<(), StorageError> {
        let resp = self
            .propose(WriteCommand::UpdateScheduleLastRun {
                schedule_id: schedule_id.to_string(),
                now: now.to_string(),
            })
            .await?;
        Self::expect_ok(resp)
    }

    /// Write
    pub async fn update_schedule_next_run(
        &self,
        schedule_id: &str,
        next_run: &str,
    ) -> Result<(), StorageError> {
        let resp = self
            .propose(WriteCommand::UpdateScheduleNextRun {
                schedule_id: schedule_id.to_string(),
                next_run: next_run.to_string(),
            })
            .await?;
        Self::expect_ok(resp)
    }

    // =======================================================================
    // Agent Ops
    // =======================================================================

    /// Read
    pub async fn list_agents(&self) -> Result<Vec<ops::agent_ops::AgentRow>, StorageError> {
        ops::agent_ops::list_agents(&self.db).await
    }

    /// Read
    pub async fn find_agents(
        &self,
        query: &str,
    ) -> Result<Vec<ops::agent_ops::AgentSearchRow>, StorageError> {
        ops::agent_ops::find_agents(&self.db, query).await
    }

    /// Write
    pub async fn spawn_agent(
        &self,
        agent_name: &str,
    ) -> Result<(String, String), StorageError> {
        let resp = self
            .propose(WriteCommand::SpawnAgent {
                agent_name: agent_name.to_string(),
            })
            .await?;
        Self::expect_id_pair(resp)
    }

    /// Write
    pub async fn kill_agent(&self, agent_id: &str) -> Result<u64, StorageError> {
        let resp = self
            .propose(WriteCommand::KillAgent {
                agent_id: agent_id.to_string(),
            })
            .await?;
        Self::expect_count(resp)
    }
}

// ---------------------------------------------------------------------------
// RuntimeStore implementation for RuneStore
// ---------------------------------------------------------------------------

#[async_trait]
impl RuntimeStore for RuneStore {
    // -----------------------------------------------------------------------
    // Sessions
    // -----------------------------------------------------------------------

    async fn create_session(
        &self,
        deployment_id: Uuid,
        tenant_id: Option<&str>,
        routing_key: Option<&str>,
    ) -> Result<Uuid, RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::CreateSession {
                deployment_id,
                tenant_id: tenant_id.map(String::from),
                routing_key: routing_key.map(String::from),
            })
            .await?;
        Self::expect_id(resp).map_err(Into::into)
    }

    async fn load_session_messages(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionMessage>, RuntimeStoreError> {
        ops::sessions::load_messages(&self.db, session_id)
            .await
            .map_err(Into::into)
    }

    async fn append_session_message(
        &self,
        session_id: Uuid,
        role: &str,
        content: &serde_json::Value,
        step: i64,
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::AppendSessionMessage {
                session_id,
                role: role.to_string(),
                content: content.clone(),
                step,
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

    async fn close_session(&self, session_id: Uuid) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::CloseSession { id: session_id })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

    async fn get_session_deployment_id(
        &self,
        session_id: Uuid,
    ) -> Result<Option<Uuid>, RuntimeStoreError> {
        ops::sessions::get_deployment_id(&self.db, session_id)
            .await
            .map_err(Into::into)
    }

    async fn get_session_assigned_replica(
        &self,
        session_id: Uuid,
    ) -> Result<Option<Uuid>, RuntimeStoreError> {
        ops::sessions::get_assigned_replica(&self.db, session_id)
            .await
            .map_err(Into::into)
    }

    async fn assign_session_replica(
        &self,
        session_id: Uuid,
        replica_id: Uuid,
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::AssignSessionReplica {
                session_id,
                replica_id,
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

    async fn checkpoint_session(
        &self,
        session_id: Uuid,
        messages: &[SessionMessage],
        step: i64,
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::CheckpointSession {
                session_id,
                messages: messages.to_vec(),
                step,
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

    async fn restore_session(
        &self,
        session_id: Uuid,
    ) -> Result<Option<(Vec<SessionMessage>, i64)>, RuntimeStoreError> {
        ops::sessions::restore(&self.db, session_id)
            .await
            .map_err(Into::into)
    }

    async fn apply_session_restore(
        &self,
        session_id: Uuid,
        messages: &[SessionMessage],
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::ApplySessionRestore {
                session_id,
                messages: messages.to_vec(),
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

    async fn get_session_info(
        &self,
        session_id: Uuid,
    ) -> Result<Option<serde_json::Value>, RuntimeStoreError> {
        ops::sessions::get_session_info(&self.db, session_id)
            .await
            .map_err(Into::into)
    }

    // -----------------------------------------------------------------------
    // Session routing
    // -----------------------------------------------------------------------

    async fn get_session_route(
        &self,
        session_id: Uuid,
    ) -> Result<Option<Uuid>, RuntimeStoreError> {
        ops::session_route::get(&self.db, session_id)
            .await
            .map_err(Into::into)
    }

    async fn set_session_route(
        &self,
        session_id: Uuid,
        replica_id: Uuid,
        ttl_secs: Option<i64>,
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::SetSessionRoute {
                session_id,
                replica_id,
                ttl_secs,
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

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
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::InsertRequest {
                request_id,
                session_id,
                deployment_id,
                replica_id,
                payload: payload.clone(),
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

    async fn update_request_completed(
        &self,
        request_id: Uuid,
        output: &serde_json::Value,
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::UpdateRequestCompleted {
                request_id,
                output: output.clone(),
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

    async fn update_request_status(
        &self,
        request_id: Uuid,
        status: &str,
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::UpdateRequestStatus {
                request_id,
                status: status.to_string(),
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

    async fn get_request_response_summary(
        &self,
        request_id: &str,
    ) -> Result<Option<String>, RuntimeStoreError> {
        ops::requests::get_response_summary(&self.db, request_id)
            .await
            .map_err(Into::into)
    }

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
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::InsertA2ATask {
                task_id: task_id.to_string(),
                context_id: context_id.to_string(),
                request_id: request_id.to_string(),
                session_id: session_id.to_string(),
                agent_name: agent_name.to_string(),
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

    async fn update_a2a_task_state(
        &self,
        task_id: &str,
        state: &str,
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::UpdateA2ATaskState {
                task_id: task_id.to_string(),
                state: state.to_string(),
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

    async fn get_a2a_task(
        &self,
        task_id: &str,
    ) -> Result<Option<A2aTaskRow>, RuntimeStoreError> {
        ops::a2a::get_task(&self.db, task_id)
            .await
            .map_err(Into::into)
    }

    async fn cancel_a2a_task(&self, task_id: &str) -> Result<u64, RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::CancelA2ATask {
                task_id: task_id.to_string(),
            })
            .await?;
        Self::expect_count(resp).map_err(Into::into)
    }

    async fn load_a2a_history(
        &self,
        session_id: &str,
        limit: i64,
    ) -> Result<Vec<HistoryMessage>, RuntimeStoreError> {
        ops::a2a::load_history(&self.db, session_id, limit)
            .await
            .map_err(Into::into)
    }

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
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::InsertToolInvocation {
                invocation_id: invocation_id.to_string(),
                request_id,
                tool_name: tool_name.to_string(),
                tool_runtime: tool_runtime.to_string(),
                input_payload: input_payload.to_string(),
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

    async fn update_tool_invocation_completed(
        &self,
        invocation_id: &str,
        output_payload: &str,
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::UpdateToolInvocationCompleted {
                invocation_id: invocation_id.to_string(),
                output_payload: output_payload.to_string(),
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

    async fn update_tool_invocation_failed(
        &self,
        invocation_id: &str,
        error_message: &str,
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::UpdateToolInvocationFailed {
                invocation_id: invocation_id.to_string(),
                error_message: error_message.to_string(),
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

    async fn insert_agent_call(
        &self,
        call_id: &str,
        caller_task_id: &str,
        caller_agent: &str,
        callee_agent: &str,
        callee_endpoint: &str,
        depth: i64,
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::InsertAgentCall {
                call_id: call_id.to_string(),
                caller_task_id: caller_task_id.to_string(),
                caller_agent: caller_agent.to_string(),
                callee_agent: callee_agent.to_string(),
                callee_endpoint: callee_endpoint.to_string(),
                depth,
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

    async fn update_agent_call_callee_task(
        &self,
        call_id: &str,
        callee_task_id: &str,
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::UpdateAgentCallCalleeTask {
                call_id: call_id.to_string(),
                callee_task_id: callee_task_id.to_string(),
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }

    async fn update_agent_call_completed(
        &self,
        call_id: &str,
        latency_ms: i64,
    ) -> Result<(), RuntimeStoreError> {
        let resp = self
            .propose(WriteCommand::UpdateAgentCallCompleted {
                call_id: call_id.to_string(),
                latency_ms,
            })
            .await?;
        Self::expect_ok(resp).map_err(Into::into)
    }
}
