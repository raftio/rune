use std::sync::Arc;
use std::time::Duration;

use rune_storage::RuneStore;
use tracing::{error, info, warn};

use crate::backend::RuntimeBackend;

pub struct ReconcileLoop {
    store: Arc<RuneStore>,
    backend: Arc<dyn RuntimeBackend>,
    interval: Duration,
    stale_threshold_secs: i64,
}

impl ReconcileLoop {
    pub fn new(store: Arc<RuneStore>, backend: Arc<dyn RuntimeBackend>) -> Self {
        Self {
            store,
            backend,
            interval: Duration::from_secs(10),
            stale_threshold_secs: 30,
        }
    }

    pub async fn run(self) {
        if let Err(e) = self.reconcile_once().await {
            error!("Initial reconcile failed: {e}");
        }

        let mut ticker = tokio::time::interval(self.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(e) = self.reconcile_once().await {
                error!("Reconcile cycle failed: {e}");
            }
        }
    }

    async fn reconcile_once(&self) -> anyhow::Result<()> {
        let stale = self.store.mark_stale_replicas_failed(self.stale_threshold_secs).await?;
        if stale > 0 {
            warn!("Marked {stale} stale replica(s) as failed");
        }

        let deployments = self.store.get_schedulable_deployments().await?;

        for dep in deployments {
            let healthy = self.store.count_healthy_replicas(dep.id).await?;
            let desired = dep.desired_replicas as i64;

            if healthy < desired {
                let to_start = desired - healthy;
                info!(deployment_id = %dep.id, "Starting {to_start} replica(s) (have {healthy}, want {desired})");
                for _ in 0..to_start {
                    match self.backend.start_replica(&dep).await {
                        Ok(replica) => {
                            info!(replica_id = %replica.id, "Replica started");
                            let _ = self.store.set_deployment_active(dep.id).await;
                        }
                        Err(e) => error!("Failed to start replica for {}: {e}", dep.id),
                    }
                }
            } else if healthy > desired {
                let excess = healthy - desired;
                info!(deployment_id = %dep.id, "Draining {excess} excess replica(s)");
                let ids = self.store.get_drainable_replica_ids(dep.id, excess).await?;
                for id in ids {
                    if let Err(e) = self.backend.drain_replica(id).await {
                        error!("Failed to drain replica {id}: {e}");
                    }
                }
            }
        }

        Ok(())
    }
}
