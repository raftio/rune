use std::sync::Arc;
use uuid::Uuid;

use rune_storage::{RuneStore, RuntimeStore};
use crate::error::RuntimeError;

pub struct ReplicaRouter {
    store: Arc<RuneStore>,
}

impl ReplicaRouter {
    pub fn new(store: Arc<RuneStore>) -> Self {
        Self { store }
    }

    pub async fn acquire(
        &self,
        deployment_id: Uuid,
        session_id: Uuid,
    ) -> Result<ReplicaLease, RuntimeError> {
        // Try cache first (session_route).
        if let Ok(Some(assigned)) = self.store.get_session_route(session_id).await {
            if self.store.is_replica_available(assigned).await? {
                self.store.increment_replica_load(assigned).await?;
                let _ = self.store.set_session_route(session_id, assigned, None).await;
                return Ok(ReplicaLease {
                    replica_id: assigned,
                    store: self.store.clone(),
                    released: false,
                });
            }
        }

        // Try session affinity from DB.
        if let Some(assigned) = self.store.get_session_assigned_replica(session_id).await? {
            if self.store.is_replica_available(assigned).await? {
                self.store.increment_replica_load(assigned).await?;
                let _ = self.store.set_session_route(session_id, assigned, None).await;
                return Ok(ReplicaLease {
                    replica_id: assigned,
                    store: self.store.clone(),
                    released: false,
                });
            }
        }

        // Least-loaded fallback.
        let replica_id = self.store
            .select_least_loaded_replica(deployment_id)
            .await?
            .ok_or_else(|| {
                RuntimeError::Backend(format!(
                    "no healthy replica available for deployment {deployment_id}"
                ))
            })?;

        self.store.increment_replica_load(replica_id).await?;
        self.store.assign_session_replica(session_id, replica_id).await?;
        let _ = self.store.set_session_route(session_id, replica_id, None).await;

        Ok(ReplicaLease {
            replica_id,
            store: self.store.clone(),
            released: false,
        })
    }
}

pub struct ReplicaLease {
    pub replica_id: Uuid,
    store: Arc<RuneStore>,
    released: bool,
}

impl ReplicaLease {
    pub async fn release(mut self) {
        self.released = true;
        if let Err(e) = self.store.decrement_replica_load(self.replica_id).await {
            tracing::error!(
                replica_id = %self.replica_id,
                error = %e,
                "failed to release replica lease — load counter may be stuck"
            );
        }
    }
}

impl Drop for ReplicaLease {
    fn drop(&mut self) {
        if !self.released {
            let store = self.store.clone();
            let id = self.replica_id;
            tokio::spawn(async move {
                if let Err(e) = store.decrement_replica_load(id).await {
                    tracing::error!(
                        replica_id = %id,
                        error = %e,
                        "ReplicaLease dropped without explicit release — load counter may be stuck"
                    );
                }
            });
        }
    }
}
