use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::sync::Arc;

use openraft::storage::{LogState, RaftLogReader, RaftSnapshotBuilder, RaftStorage};
use openraft::{
    Entry, EntryPayload, LogId, Snapshot, SnapshotMeta, StorageError, StorageIOError,
    StoredMembership, Vote,
};
use sqlx::SqlitePool;
use tokio::sync::RwLock;

use super::types::{RuneTypeConfig, WriteCommand, WriteResponse};
use crate::ops;

/// Combined Raft log + state machine storage backed by SQLite.
///
/// Log entries are held in-memory (sufficient for development; a durable
/// SQLite-backed log can be added later). State machine mutations go straight
/// to the application SQLite database via `ops::*`.
pub struct RuneRaftStore {
    inner: Arc<RwLock<StoreInner>>,
}

struct StoredSnapshot {
    meta: SnapshotMeta<u64, openraft::BasicNode>,
    data: Vec<u8>,
}

struct StoreInner {
    db: SqlitePool,

    // --- log ---
    last_purged: Option<LogId<u64>>,
    log: BTreeMap<u64, Entry<RuneTypeConfig>>,
    committed: Option<LogId<u64>>,
    vote: Option<Vote<u64>>,

    // --- state machine ---
    last_applied_log: Option<LogId<u64>>,
    last_membership: StoredMembership<u64, openraft::BasicNode>,
    snapshot_idx: u64,
    last_snapshot: Option<StoredSnapshot>,
}

impl RuneRaftStore {
    pub fn new(db: SqlitePool) -> Self {
        Self {
            inner: Arc::new(RwLock::new(StoreInner {
                db,
                last_purged: None,
                log: BTreeMap::new(),
                committed: None,
                vote: None,
                last_applied_log: None,
                last_membership: StoredMembership::default(),
                snapshot_idx: 0,
                last_snapshot: None,
            })),
        }
    }
}

impl Clone for RuneRaftStore {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

pub async fn apply_command(
    db: &SqlitePool,
    cmd: WriteCommand,
) -> Result<WriteResponse, crate::error::StorageError> {
    match cmd {
        // -- Deployments --
        WriteCommand::CreateDeployment(input) => {
            let id = ops::deployments::create(db, &input).await?;
            Ok(WriteResponse::Id(id))
        }
        WriteCommand::SetDeploymentActive { id } => {
            ops::deployments::set_active(db, id).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::ScaleDeployment { id, replicas } => {
            let n = ops::deployments::scale(db, id, replicas).await?;
            Ok(WriteResponse::Count(n))
        }
        WriteCommand::DeleteDeployment { id } => {
            let n = ops::deployments::delete(db, id).await?;
            Ok(WriteResponse::Count(n))
        }
        WriteCommand::DeleteDeploymentForce { id } => {
            let n = ops::deployments::delete_force(db, id).await?;
            Ok(WriteResponse::Count(n))
        }

        // -- Replicas --
        WriteCommand::InsertReplica(r) => {
            let id = ops::replicas::insert(db, &r).await?;
            Ok(WriteResponse::Id(id))
        }
        WriteCommand::SetReplicaReady { id } => {
            ops::replicas::set_ready(db, id).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::SetReplicaState { id, state } => {
            ops::replicas::set_state(db, id, state).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::HeartbeatReplica { id } => {
            ops::replicas::heartbeat(db, id).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::MarkStaleReplicasFailed { threshold_secs } => {
            let n = ops::replicas::mark_stale_failed(db, threshold_secs).await?;
            Ok(WriteResponse::Count(n))
        }
        WriteCommand::IncrementReplicaLoad { id } => {
            ops::replicas::increment_load(db, id).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::DecrementReplicaLoad { id } => {
            ops::replicas::decrement_load(db, id).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::UpdateReplicaBackendInstanceId { id, container_id } => {
            ops::replicas::update_backend_instance_id(db, id, &container_id).await?;
            Ok(WriteResponse::Ok)
        }

        // -- Sessions --
        WriteCommand::CreateSession {
            deployment_id,
            tenant_id,
            routing_key,
        } => {
            let id = ops::sessions::create(
                db,
                deployment_id,
                tenant_id.as_deref(),
                routing_key.as_deref(),
            )
            .await?;
            Ok(WriteResponse::Id(id))
        }
        WriteCommand::CloseSession { id } => {
            ops::sessions::close(db, id).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::AppendSessionMessage {
            session_id,
            role,
            content,
            step,
        } => {
            ops::sessions::append_message(db, session_id, &role, &content, step).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::AssignSessionReplica {
            session_id,
            replica_id,
        } => {
            ops::sessions::assign_replica(db, session_id, replica_id).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::CheckpointSession {
            session_id,
            messages,
            step,
        } => {
            ops::sessions::checkpoint(db, session_id, &messages, step).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::ApplySessionRestore {
            session_id,
            messages,
        } => {
            ops::sessions::apply_restore(db, session_id, &messages).await?;
            Ok(WriteResponse::Ok)
        }

        // -- Requests --
        WriteCommand::InsertRequest {
            request_id,
            session_id,
            deployment_id,
            replica_id,
            payload,
        } => {
            ops::requests::insert(db, request_id, session_id, deployment_id, replica_id, &payload)
                .await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::UpdateRequestCompleted {
            request_id,
            output,
        } => {
            ops::requests::update_completed(db, request_id, &output).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::UpdateRequestStatus {
            request_id,
            status,
        } => {
            ops::requests::update_status(db, request_id, &status).await?;
            Ok(WriteResponse::Ok)
        }

        // -- Tools --
        WriteCommand::InsertToolInvocation {
            invocation_id,
            request_id,
            tool_name,
            tool_runtime,
            input_payload,
        } => {
            ops::tools::insert_invocation(
                db,
                &invocation_id,
                request_id,
                &tool_name,
                &tool_runtime,
                &input_payload,
            )
            .await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::UpdateToolInvocationCompleted {
            invocation_id,
            output_payload,
        } => {
            ops::tools::update_invocation_completed(db, &invocation_id, &output_payload).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::UpdateToolInvocationFailed {
            invocation_id,
            error_message,
        } => {
            ops::tools::update_invocation_failed(db, &invocation_id, &error_message).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::InsertAgentCall {
            call_id,
            caller_task_id,
            caller_agent,
            callee_agent,
            callee_endpoint,
            depth,
        } => {
            ops::tools::insert_agent_call(
                db,
                &call_id,
                &caller_task_id,
                &caller_agent,
                &callee_agent,
                &callee_endpoint,
                depth,
            )
            .await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::UpdateAgentCallCalleeTask {
            call_id,
            callee_task_id,
        } => {
            ops::tools::update_agent_call_callee_task(db, &call_id, &callee_task_id).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::UpdateAgentCallCompleted {
            call_id,
            latency_ms,
        } => {
            ops::tools::update_agent_call_completed(db, &call_id, latency_ms).await?;
            Ok(WriteResponse::Ok)
        }

        // -- A2A --
        WriteCommand::InsertA2ATask {
            task_id,
            context_id,
            request_id,
            session_id,
            agent_name,
        } => {
            ops::a2a::insert_task(db, &task_id, &context_id, &request_id, &session_id, &agent_name)
                .await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::UpdateA2ATaskState { task_id, state } => {
            ops::a2a::update_state(db, &task_id, &state).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::CancelA2ATask { task_id } => {
            let n = ops::a2a::cancel_task(db, &task_id).await?;
            Ok(WriteResponse::Count(n))
        }

        // -- Cache --
        WriteCommand::SetSessionRoute {
            session_id,
            replica_id,
            ttl_secs,
        } => {
            ops::session_route::set(db, session_id, replica_id, ttl_secs).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::RateLimitCheck {
            key,
            window_seconds,
            max_count,
        } => {
            let ok = ops::cache::rate_limit_check(db, &key, window_seconds, max_count).await?;
            Ok(WriteResponse::RateLimitResult(ok))
        }
        WriteCommand::CleanupExpiredCache => {
            let n = ops::cache::cleanup_expired(db).await?;
            Ok(WriteResponse::Count(n))
        }

        // -- Registry --
        WriteCommand::RegisterAgentVersion(input) => {
            let row = ops::registry::register(db, &input).await?;
            Ok(WriteResponse::StringId(row.id))
        }

        // -- Policy --
        WriteCommand::AuditPolicyDecision {
            kind,
            resource,
            allowed,
            reason,
            request_id,
        } => {
            ops::policy::audit_decision(
                db,
                &kind,
                &resource,
                allowed,
                reason.as_deref(),
                request_id.as_deref(),
            )
            .await?;
            Ok(WriteResponse::Ok)
        }

        // -- Schedules --
        WriteCommand::UpdateScheduleLastRun { schedule_id, now } => {
            ops::schedules::update_last_run(db, &schedule_id, &now).await?;
            Ok(WriteResponse::Ok)
        }
        WriteCommand::UpdateScheduleNextRun {
            schedule_id,
            next_run,
        } => {
            ops::schedules::update_next_run(db, &schedule_id, &next_run).await?;
            Ok(WriteResponse::Ok)
        }

        // -- Agent Ops --
        WriteCommand::SpawnAgent { agent_name } => {
            let (id, dep_id) = ops::agent_ops::spawn_agent(db, &agent_name).await?;
            Ok(WriteResponse::IdPair(id, dep_id))
        }
        WriteCommand::KillAgent { agent_id } => {
            let n = ops::agent_ops::kill_agent(db, &agent_id).await?;
            Ok(WriteResponse::Count(n))
        }
    }
}

// ---------------------------------------------------------------------------
// RaftLogReader
// ---------------------------------------------------------------------------

impl RaftLogReader<RuneTypeConfig> for RuneRaftStore {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<RuneTypeConfig>>, StorageError<u64>> {
        let inner = self.inner.read().await;
        let entries: Vec<_> = inner.log.range(range).map(|(_, e)| e.clone()).collect();
        Ok(entries)
    }
}

// ---------------------------------------------------------------------------
// RaftStorage (v1 combined trait)
// ---------------------------------------------------------------------------

impl RaftStorage<RuneTypeConfig> for RuneRaftStore {
    type LogReader = Self;
    type SnapshotBuilder = RuneSnapshotBuilder;

    // --- vote ---

    async fn save_vote(&mut self, vote: &Vote<u64>) -> Result<(), StorageError<u64>> {
        let mut inner = self.inner.write().await;
        inner.vote = Some(*vote);
        Ok(())
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<u64>>, StorageError<u64>> {
        let inner = self.inner.read().await;
        Ok(inner.vote)
    }

    // --- log ---

    async fn get_log_state(&mut self) -> Result<LogState<RuneTypeConfig>, StorageError<u64>> {
        let inner = self.inner.read().await;
        let last_log_id = inner.log.values().next_back().map(|e| e.log_id);
        Ok(LogState {
            last_purged_log_id: inner.last_purged,
            last_log_id: last_log_id.or(inner.last_purged),
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn append_to_log<I>(
        &mut self,
        entries: I,
    ) -> Result<(), StorageError<u64>>
    where
        I: IntoIterator<Item = Entry<RuneTypeConfig>> + Send,
    {
        let mut inner = self.inner.write().await;
        for entry in entries {
            inner.log.insert(entry.log_id.index, entry);
        }
        Ok(())
    }

    async fn delete_conflict_logs_since(
        &mut self,
        log_id: LogId<u64>,
    ) -> Result<(), StorageError<u64>> {
        let mut inner = self.inner.write().await;
        let keys: Vec<u64> = inner.log.range(log_id.index..).map(|(k, _)| *k).collect();
        for k in keys {
            inner.log.remove(&k);
        }
        Ok(())
    }

    async fn purge_logs_upto(&mut self, log_id: LogId<u64>) -> Result<(), StorageError<u64>> {
        let mut inner = self.inner.write().await;
        let keys: Vec<u64> = inner.log.range(..=log_id.index).map(|(k, _)| *k).collect();
        for k in keys {
            inner.log.remove(&k);
        }
        inner.last_purged = Some(log_id);
        Ok(())
    }

    // --- state machine ---

    async fn last_applied_state(
        &mut self,
    ) -> Result<
        (
            Option<LogId<u64>>,
            StoredMembership<u64, openraft::BasicNode>,
        ),
        StorageError<u64>,
    > {
        let inner = self.inner.read().await;
        Ok((inner.last_applied_log, inner.last_membership.clone()))
    }

    async fn apply_to_state_machine(
        &mut self,
        entries: &[Entry<RuneTypeConfig>],
    ) -> Result<Vec<WriteResponse>, StorageError<u64>> {
        let mut inner = self.inner.write().await;
        let mut results = Vec::new();

        for entry in entries {
            inner.last_applied_log = Some(entry.log_id);

            match &entry.payload {
                EntryPayload::Normal(cmd) => {
                    let resp = apply_command(&inner.db, cmd.clone())
                        .await
                        .map_err(|e| StorageError::IO {
                            source: StorageIOError::write(&e),
                        })?;
                    results.push(resp);
                }
                EntryPayload::Membership(mem) => {
                    inner.last_membership =
                        StoredMembership::new(Some(entry.log_id), mem.clone());
                    results.push(WriteResponse::Ok);
                }
                EntryPayload::Blank => {
                    results.push(WriteResponse::Ok);
                }
            }
        }

        Ok(results)
    }

    // --- snapshots ---

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        let inner = self.inner.read().await;
        RuneSnapshotBuilder {
            db: inner.db.clone(),
            last_applied_log: inner.last_applied_log,
            last_membership: inner.last_membership.clone(),
            snapshot_idx: inner.snapshot_idx + 1,
        }
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<Cursor<Vec<u8>>>, StorageError<u64>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<u64, openraft::BasicNode>,
        snapshot: Box<Cursor<Vec<u8>>>,
    ) -> Result<(), StorageError<u64>> {
        let mut inner = self.inner.write().await;
        inner.last_applied_log = meta.last_log_id;
        inner.last_membership = meta.last_membership.clone();

        let data = snapshot.into_inner();
        if !data.is_empty() {
            tracing::info!(bytes = data.len(), "installing snapshot from leader");

            // Write snapshot to a secure temp file (O_EXCL via tempfile crate
            // prevents symlink attacks on /tmp).
            match tempfile::Builder::new()
                .prefix("rune-snap-install-")
                .suffix(".db")
                .tempfile()
            {
                Ok(mut tmp_file) => {
                    use std::io::Write;
                    if let Err(e) = tmp_file.write_all(&data) {
                        tracing::error!(error = %e, "failed to write snapshot to temp file");
                    } else {
                        tracing::info!(
                            path = %tmp_file.path().display(),
                            "snapshot written — manual restore needed for full consistency"
                        );
                    }
                    // File is automatically deleted when tmp_file is dropped
                }
                Err(e) => {
                    tracing::error!(error = %e, "failed to create temp file for snapshot");
                }
            }
        }

        // Persist the snapshot metadata so get_current_snapshot can return it
        inner.last_snapshot = Some(StoredSnapshot {
            meta: meta.clone(),
            data,
        });

        Ok(())
    }

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<RuneTypeConfig>>, StorageError<u64>> {
        let inner = self.inner.read().await;
        Ok(inner.last_snapshot.as_ref().map(|s| Snapshot {
            meta: s.meta.clone(),
            snapshot: Box::new(Cursor::new(s.data.clone())),
        }))
    }

    async fn save_committed(
        &mut self,
        committed: Option<LogId<u64>>,
    ) -> Result<(), StorageError<u64>> {
        let mut inner = self.inner.write().await;
        inner.committed = committed;
        Ok(())
    }

    async fn read_committed(&mut self) -> Result<Option<LogId<u64>>, StorageError<u64>> {
        let inner = self.inner.read().await;
        Ok(inner.committed)
    }
}

// ---------------------------------------------------------------------------
// Snapshot builder — uses SQLite VACUUM INTO for a consistent backup
// ---------------------------------------------------------------------------

pub struct RuneSnapshotBuilder {
    db: SqlitePool,
    last_applied_log: Option<LogId<u64>>,
    last_membership: StoredMembership<u64, openraft::BasicNode>,
    snapshot_idx: u64,
}

impl RaftSnapshotBuilder<RuneTypeConfig> for RuneSnapshotBuilder {
    async fn build_snapshot(&mut self) -> Result<Snapshot<RuneTypeConfig>, StorageError<u64>> {
        let snapshot_id = format!(
            "{}-{}",
            self.last_applied_log
                .map(|l| l.index.to_string())
                .unwrap_or_default(),
            self.snapshot_idx,
        );

        let meta = SnapshotMeta {
            last_log_id: self.last_applied_log,
            last_membership: self.last_membership.clone(),
            snapshot_id,
        };

        // Use tempfile crate for secure temp file creation (O_EXCL, no symlink race).
        // SQLite VACUUM INTO doesn't support parameterized paths, so we
        // validate the path contains only safe characters.
        let tmp_file = tempfile::Builder::new()
            .prefix("rune-snapshot-")
            .suffix(".db")
            .tempfile()
            .map_err(|e| StorageError::IO {
                source: StorageIOError::write(&e),
            })?;
        let tmp_path = tmp_file.path().to_path_buf();
        let tmp_str = tmp_path.to_string_lossy().to_string();

        // Reject paths with characters that could break the SQL literal
        if tmp_str.contains('\'') || tmp_str.contains('\\') || tmp_str.contains(';') {
            return Err(StorageError::IO {
                source: StorageIOError::write(&std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "temp path contains unsafe characters for VACUUM INTO",
                )),
            });
        }

        // Drop the tempfile handle so SQLite can write to the path
        let _ = tmp_file.keep();

        let data = match sqlx::query(&format!("VACUUM INTO '{}'", tmp_str))
            .execute(&self.db)
            .await
        {
            Ok(_) => {
                let bytes = tokio::fs::read(&tmp_path).await.map_err(|e| {
                    StorageError::IO {
                        source: StorageIOError::read(&e),
                    }
                })?;
                let _ = tokio::fs::remove_file(&tmp_path).await;
                tracing::info!(bytes = bytes.len(), "built snapshot via VACUUM INTO");
                bytes
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                tracing::warn!(error = %e, "VACUUM INTO failed, using empty snapshot");
                Vec::new()
            }
        };

        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}
