mod manifests;
mod wait;

use std::sync::Arc;

use async_trait::async_trait;
use k8s_openapi::api::apps::v1::Deployment as K8sDeployment;
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use k8s_openapi::api::core::v1::{Pod, Service};
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use kube::api::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::{Api, Client};
use rune_env::{AgentEnv, PlatformEnv};
use rune_storage::RuneStore;
use uuid::Uuid;

use manifests::ManifestParams;
use rune_runtime::{
    backend::ReplicaStats,
    error::RuntimeError,
    models::{BackendType, Deployment, HealthStatus, NewReplica, Replica, ReplicaState},
    signature,
    RuntimeBackend,
};

const DRAIN_TIMEOUT_SECS: u64 = 30;

pub struct KubernetesBackend {
    client: Client,
    store: Arc<RuneStore>,
    namespace: String,
    env: Arc<PlatformEnv>,
}

impl KubernetesBackend {
    pub async fn new(store: Arc<RuneStore>, namespace: String, env: Arc<PlatformEnv>) -> anyhow::Result<Self> {
        let client = Client::try_default()
            .await
            .map_err(|e| anyhow::anyhow!("failed to create K8s client: {e}"))?;

        Ok(Self {
            client,
            store,
            namespace,
            env,
        })
    }

    fn resource_name(replica_id: &str) -> String {
        format!("rune-{}", &replica_id[..8.min(replica_id.len())])
    }

    fn selector_for_replica(replica_id: &str) -> String {
        format!(
            "app.kubernetes.io/managed-by=rune,rune.io/replica-id={replica_id}"
        )
    }
}

#[async_trait]
impl RuntimeBackend for KubernetesBackend {
    async fn start_replica(&self, deployment: &Deployment) -> Result<Replica, RuntimeError> {
        let image_info = self
            .store
            .get_agent_version_image(deployment.agent_version_id)
            .await
            .map_err(|e| RuntimeError::Backend(format!("storage: {e}")))?
            .ok_or_else(|| RuntimeError::Backend("agent version image not found".into()))?;

        let image = format!("{}@{}", image_info.image_ref, image_info.image_digest);

        signature::verify_image_signature(
            &image_info.image_ref,
            &image_info.image_digest,
            image_info.signature_ref.as_deref(),
        )
        .await?;

        let replica_id = self
            .store
            .insert_replica(&NewReplica {
                deployment_id: deployment.id,
                backend_type: BackendType::Kubernetes,
                backend_instance_id: String::new(),
                concurrency_limit: deployment.concurrency_limit,
            })
            .await
            .map_err(|e| RuntimeError::Backend(format!("insert replica: {e}")))?;

        let replica_id_str = replica_id.to_string();
        let deployment_id_str = deployment.id.to_string();
        let agent_env = AgentEnv::resolve(&self.env, deployment.env_json.as_deref());
        let agent_env_json = serde_json::to_string(&agent_env.to_map()).ok();
        let config_ref = deployment.config_ref.as_deref();
        let secret_ref = deployment.secret_ref.as_deref();

        let params = ManifestParams {
            deployment_id: &deployment_id_str,
            replica_id: &replica_id_str,
            image: &image,
            namespace: &self.namespace,
            min_replicas: deployment.min_replicas,
            max_replicas: deployment.max_replicas,
            config_ref,
            secret_ref,
            env_json: agent_env_json.as_deref(),
        };

        let pp = PostParams::default();

        let deployments_api: Api<K8sDeployment> =
            Api::namespaced(self.client.clone(), &self.namespace);
        let dep = manifests::deployment_manifest(&params);
        deployments_api
            .create(&pp, &dep)
            .await
            .map_err(|e| RuntimeError::Backend(format!("create Deployment: {e}")))?;

        let services_api: Api<Service> = Api::namespaced(self.client.clone(), &self.namespace);
        let svc = manifests::service_manifest(&params);
        services_api
            .create(&pp, &svc)
            .await
            .map_err(|e| RuntimeError::Backend(format!("create Service: {e}")))?;

        let pdb_api: Api<PodDisruptionBudget> =
            Api::namespaced(self.client.clone(), &self.namespace);
        let pdb = manifests::pdb_manifest(&params);
        pdb_api
            .create(&pp, &pdb)
            .await
            .map_err(|e| RuntimeError::Backend(format!("create PDB: {e}")))?;

        let hpa_api: Api<HorizontalPodAutoscaler> =
            Api::namespaced(self.client.clone(), &self.namespace);
        let hpa = manifests::hpa_manifest(&params);
        hpa_api
            .create(&pp, &hpa)
            .await
            .map_err(|e| RuntimeError::Backend(format!("create HPA: {e}")))?;

        let pods_api: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);
        let label_sel = Self::selector_for_replica(&replica_id_str);
        let pod_name = wait::wait_for_pod_ready(&pods_api, &label_sel, None).await?;

        self.store
            .update_replica_backend_instance_id(replica_id, &pod_name)
            .await
            .map_err(|e| RuntimeError::Backend(format!("update replica: {e}")))?;

        self.store.set_replica_ready(replica_id).await.map_err(|e| {
            RuntimeError::Backend(format!("set ready: {e}"))
        })?;

        let resource_name = Self::resource_name(&replica_id_str);
        let endpoint = format!(
            "{resource_name}.{}.svc.cluster.local:8080",
            self.namespace
        );

        tracing::info!(
            replica_id = %replica_id,
            pod = %pod_name,
            deployment_id = %deployment.id,
            "Kubernetes replica started"
        );

        Ok(Replica {
            id: replica_id,
            deployment_id: deployment.id,
            backend_type: BackendType::Kubernetes,
            backend_instance_id: pod_name,
            node_name: None,
            endpoint: Some(endpoint),
            state: ReplicaState::Ready,
            concurrency_limit: deployment.concurrency_limit,
            current_load: 0,
            started_at: Some(chrono::Utc::now()),
            last_heartbeat_at: Some(chrono::Utc::now()),
        })
    }

    async fn drain_replica(&self, replica_id: Uuid) -> Result<(), RuntimeError> {
        self.store
            .set_replica_state(replica_id, ReplicaState::Draining)
            .await?;

        let replica_id_str = replica_id.to_string();
        let name = Self::resource_name(&replica_id_str);

        let deployments_api: Api<K8sDeployment> =
            Api::namespaced(self.client.clone(), &self.namespace);

        let patch = serde_json::json!({
            "spec": { "replicas": 0 }
        });
        let _ = deployments_api
            .patch(
                &name,
                &PatchParams::apply("rune-drain"),
                &Patch::Merge(&patch),
            )
            .await
            .map_err(|e| RuntimeError::Backend(format!("scale to 0: {e}")))?;

        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_secs(DRAIN_TIMEOUT_SECS);
        let pods_api: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);
        let label_sel = Self::selector_for_replica(&replica_id_str);

        loop {
            let lp = ListParams::default().labels(&label_sel);
            let pod_list = pods_api
                .list(&lp)
                .await
                .map_err(|e| RuntimeError::Backend(format!("list pods: {e}")))?;

            if pod_list.items.is_empty() {
                break;
            }

            if tokio::time::Instant::now() >= deadline {
                tracing::warn!(
                    replica_id = %replica_id,
                    "Drain grace period expired with pods still running"
                );
                break;
            }

            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }

        self.stop_replica(replica_id).await
    }

    async fn stop_replica(&self, replica_id: Uuid) -> Result<(), RuntimeError> {
        let replica_id_str = replica_id.to_string();
        let name = Self::resource_name(&replica_id_str);
        let dp = DeleteParams::default();

        let deployments_api: Api<K8sDeployment> =
            Api::namespaced(self.client.clone(), &self.namespace);
        let _ = deployments_api.delete(&name, &dp).await;

        let services_api: Api<Service> = Api::namespaced(self.client.clone(), &self.namespace);
        let _ = services_api.delete(&name, &dp).await;

        let pdb_api: Api<PodDisruptionBudget> =
            Api::namespaced(self.client.clone(), &self.namespace);
        let _ = pdb_api.delete(&format!("{name}-pdb"), &dp).await;

        let hpa_api: Api<HorizontalPodAutoscaler> =
            Api::namespaced(self.client.clone(), &self.namespace);
        let _ = hpa_api.delete(&format!("{name}-hpa"), &dp).await;

        self.store
            .set_replica_state(replica_id, ReplicaState::Stopped)
            .await?;

        tracing::info!(replica_id = %replica_id, "Kubernetes replica stopped");
        Ok(())
    }

    async fn health(&self, replica_id: Uuid) -> Result<HealthStatus, RuntimeError> {
        let replica_id_str = replica_id.to_string();
        let pods_api: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);
        let label_sel = Self::selector_for_replica(&replica_id_str);
        let lp = ListParams::default().labels(&label_sel);

        let pod_list = pods_api
            .list(&lp)
            .await
            .map_err(|e| RuntimeError::Backend(format!("list pods: {e}")))?;

        let Some(pod) = pod_list.items.first() else {
            return Ok(HealthStatus {
                healthy: false,
                message: Some("no pods found".into()),
            });
        };

        let (healthy, message) = wait::check_pod_healthy(pod);
        Ok(HealthStatus { healthy, message })
    }

    async fn stats(&self, replica_id: Uuid) -> Result<ReplicaStats, RuntimeError> {
        let current_load = self
            .store
            .get_replica_current_load(replica_id)
            .await
            .unwrap_or(0) as u32;

        // K8s metrics-server integration would go here for memory_bytes;
        // for now we return None like the WASM backend.
        Ok(ReplicaStats {
            current_load,
            memory_bytes: None,
        })
    }
}
