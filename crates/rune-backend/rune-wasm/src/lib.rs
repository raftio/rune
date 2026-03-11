mod instance;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use rune_env::PlatformEnv;
use rune_storage::RuneStore;
use tokio::sync::RwLock;
use uuid::Uuid;
use wasmtime::Engine;

use rune_runtime::{
    backend::ReplicaStats,
    error::RuntimeError,
    models::{BackendType, Deployment, HealthStatus, NewReplica, Replica, ReplicaState},
    RuntimeBackend,
};

use instance::{spawn_replica, WasmReplicaHandle};

pub struct WasmBackend {
    engine: Engine,
    store: Arc<RuneStore>,
    replicas: Arc<RwLock<HashMap<Uuid, WasmReplicaHandle>>>,
    env: Arc<PlatformEnv>,
}

impl WasmBackend {
    pub fn new(store: Arc<RuneStore>, env: Arc<PlatformEnv>) -> anyhow::Result<Self> {
        let mut config = wasmtime::Config::new();
        config.async_support(true);

        let engine = Engine::new(&config)?;

        Ok(Self {
            engine,
            store,
            replicas: Arc::new(RwLock::new(HashMap::new())),
            env,
        })
    }
}

#[async_trait]
impl RuntimeBackend for WasmBackend {
    async fn start_replica(&self, deployment: &Deployment) -> Result<Replica, RuntimeError> {
        let backend_instance_id = Uuid::new_v4().to_string();
        let replica_id = self.store
            .insert_replica(&NewReplica {
                deployment_id: deployment.id,
                backend_type: BackendType::Wasm,
                backend_instance_id: backend_instance_id.clone(),
                concurrency_limit: deployment.concurrency_limit,
            })
            .await?;

        let handle = spawn_replica(replica_id, self.engine.clone());
        self.replicas.write().await.insert(replica_id, handle);

        self.store.set_replica_ready(replica_id).await?;

        tracing::info!(
            replica_id = %replica_id,
            deployment_id = %deployment.id,
            "WASM replica registered"
        );

        Ok(Replica {
            id: replica_id,
            deployment_id: deployment.id,
            backend_type: BackendType::Wasm,
            backend_instance_id,
            node_name: None,
            endpoint: None,
            state: ReplicaState::Ready,
            concurrency_limit: deployment.concurrency_limit,
            current_load: 0,
            started_at: Some(chrono::Utc::now()),
            last_heartbeat_at: Some(chrono::Utc::now()),
        })
    }

    async fn drain_replica(&self, replica_id: Uuid) -> Result<(), RuntimeError> {
        self.store.set_replica_state(replica_id, ReplicaState::Draining).await?;
        tracing::info!(replica_id = %replica_id, "Replica marked draining");

        let grace_secs = self.env.drain_grace_secs;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(grace_secs);
        loop {
            let load = self.store.get_replica_current_load(replica_id).await.unwrap_or(0);
            if load <= 0 {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!(replica_id = %replica_id, load, "Drain grace period expired with active load");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        self.stop_replica(replica_id).await
    }

    async fn stop_replica(&self, replica_id: Uuid) -> Result<(), RuntimeError> {
        let handle = self.replicas.write().await.remove(&replica_id);
        if let Some(h) = handle {
            let _ = h.shutdown_tx.send(());
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                h.task,
            )
            .await;
        }
        self.store.set_replica_state(replica_id, ReplicaState::Stopped).await?;
        tracing::info!(replica_id = %replica_id, "Replica stopped");
        Ok(())
    }

    async fn health(&self, replica_id: Uuid) -> Result<HealthStatus, RuntimeError> {
        let replicas = self.replicas.read().await;
        match replicas.get(&replica_id) {
            Some(h) if h.is_running() => Ok(HealthStatus {
                healthy: true,
                message: None,
            }),
            Some(_) => Ok(HealthStatus {
                healthy: false,
                message: Some("Task finished unexpectedly".into()),
            }),
            None => Ok(HealthStatus {
                healthy: false,
                message: Some("No in-memory handle — replica may have been restarted".into()),
            }),
        }
    }

    async fn stats(&self, replica_id: Uuid) -> Result<ReplicaStats, RuntimeError> {
        let replicas = self.replicas.read().await;
        let running = replicas.get(&replica_id).map(|h| h.is_running()).unwrap_or(false);
        let current_load = if running {
            self.store.get_replica_current_load(replica_id).await.unwrap_or(0) as u32
        } else {
            0
        };
        Ok(ReplicaStats {
            current_load,
            memory_bytes: None,
        })
    }
}
