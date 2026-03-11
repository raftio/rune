use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use uuid::Uuid;
use wasmtime::Engine;

/// Message sent to the replica to process an invocation.
/// For now a placeholder; full invoke context would be added when gateway routes to replica.
#[derive(Debug)]
pub struct InvokeRequest {
    /// Placeholder for future: session_id, input, reply channel, etc.
    _payload: (),
}

/// An in-memory handle to a running WASM replica task.
pub struct WasmReplicaHandle {
    #[allow(dead_code)]
    pub replica_id: Uuid,
    pub shutdown_tx: oneshot::Sender<()>,
    /// Send invoke requests to the replica. Receiving end is in the task.
    #[allow(dead_code)]
    pub invoke_tx: mpsc::Sender<InvokeRequest>,
    pub task: JoinHandle<()>,
}

impl WasmReplicaHandle {
    pub fn is_running(&self) -> bool {
        !self.task.is_finished()
    }
}

/// Spawn the replica background task.
/// The task holds a request channel for agent invocations — it selects between
/// shutdown and incoming invoke requests.
pub fn spawn_replica(replica_id: Uuid, engine: Engine) -> WasmReplicaHandle {
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let (invoke_tx, mut invoke_rx) = mpsc::channel::<InvokeRequest>(32);

    let task = tokio::spawn(async move {
        tracing::info!(replica_id = %replica_id, "WASM replica started");

        tokio::select! {
            _ = shutdown_rx => {
                tracing::info!(replica_id = %replica_id, "WASM replica received shutdown signal");
            }
            _ = async {
                loop {
                    match invoke_rx.recv().await {
                        Some(_) => {
                            tracing::debug!(replica_id = %replica_id, "WASM replica received invoke (processed in gateway)");
                        }
                        None => break,
                    }
                }
            } => {}
        }

        drop(engine);
        tracing::info!(replica_id = %replica_id, "WASM replica stopped");
    });

    WasmReplicaHandle { replica_id, shutdown_tx, invoke_tx, task }
}
