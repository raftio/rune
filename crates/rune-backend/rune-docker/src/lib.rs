use std::sync::Arc;

use async_trait::async_trait;
use bollard::container::{
    Config, CreateContainerOptions, InspectContainerOptions, RemoveContainerOptions,
    StartContainerOptions, StopContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::Docker;
use futures_util::StreamExt;
use rune_env::{AgentEnv, PlatformEnv};
use rune_storage::RuneStore;
use uuid::Uuid;

use rune_runtime::{
    backend::ReplicaStats,
    error::RuntimeError,
    models::{BackendType, Deployment, HealthStatus, NewReplica, Replica, ReplicaState},
    signature,
    RuntimeBackend,
};

const CONTAINER_NAME_PREFIX: &str = "rune-agent-";
const DRAIN_TIMEOUT_SECS: u64 = 30;

pub struct DockerBackend {
    docker: Docker,
    store: Arc<RuneStore>,
    env: Arc<PlatformEnv>,
}

impl DockerBackend {
    pub fn new(store: Arc<RuneStore>, env: Arc<PlatformEnv>) -> Result<Self, bollard::errors::Error> {
        let docker = Docker::connect_with_local_defaults()?;
        Ok(Self { docker, store, env })
    }

    fn image_with_digest(image_ref: &str, image_digest: &str) -> String {
        let digest = image_digest
            .strip_prefix("sha256:")
            .map(|s| format!("sha256:{s}"))
            .unwrap_or_else(|| image_digest.to_string());
        format!("{image_ref}@{digest}")
    }
}

#[async_trait]
impl RuntimeBackend for DockerBackend {
    async fn start_replica(&self, deployment: &Deployment) -> Result<Replica, RuntimeError> {
        let image_info = self.store
            .get_agent_version_image(deployment.agent_version_id)
            .await
            .map_err(|e| RuntimeError::Backend(format!("storage: {e}")))?
            .ok_or_else(|| RuntimeError::Backend("agent version image not found".into()))?;

        let image = Self::image_with_digest(&image_info.image_ref, &image_info.image_digest);

        signature::verify_image_signature(
            &image_info.image_ref,
            &image_info.image_digest,
            image_info.signature_ref.as_deref(),
        )
        .await?;

        let mut pull_stream = self.docker.create_image(
            Some(CreateImageOptions::<String> {
                from_image: image.clone(),
                ..Default::default()
            }),
            None,
            None,
        );
        while let Some(res) = pull_stream.next().await {
            res.map_err(|e| RuntimeError::Backend(format!("pull image: {e}")))?;
        }

        let container_name = format!("{CONTAINER_NAME_PREFIX}{}", Uuid::new_v4());

        let replica_id = self.store
            .insert_replica(&NewReplica {
                deployment_id: deployment.id,
                backend_type: BackendType::Docker,
                backend_instance_id: container_name.clone(),
                concurrency_limit: deployment.concurrency_limit,
            })
            .await
            .map_err(|e| RuntimeError::Backend(format!("insert replica: {e}")))?;

        let agent_env = AgentEnv::resolve(&self.env, deployment.env_json.as_deref());

        let config = Config {
            image: Some(image),
            cmd: Some(vec![
                "agent-runtime".to_string(),
                "--agent-dir".to_string(),
                "/agent".to_string(),
            ]),
            env: Some(agent_env.to_docker_env()),
            hostname: Some(container_name.clone()),
            ..Default::default()
        };

        let create_res = self
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name: container_name.as_str(),
                    platform: None,
                }),
                config,
            )
            .await
            .map_err(|e| RuntimeError::Backend(format!("create container: {e}")))?;

        let container_id = create_res.id;

        self.docker
            .start_container(&container_id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| RuntimeError::Backend(format!("start container: {e}")))?;

        self.store
            .update_replica_backend_instance_id(replica_id, &container_id)
            .await
            .map_err(|e| RuntimeError::Backend(format!("update replica: {e}")))?;

        self.store
            .set_replica_ready(replica_id)
            .await
            .map_err(|e| RuntimeError::Backend(format!("set ready: {e}")))?;

        tracing::info!(
            replica_id = %replica_id,
            container_id = %container_id,
            deployment_id = %deployment.id,
            "Docker replica started"
        );

        Ok(Replica {
            id: replica_id,
            deployment_id: deployment.id,
            backend_type: BackendType::Docker,
            backend_instance_id: container_id,
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
        let container_id = self.store
            .get_replica_backend_instance_id(replica_id)
            .await
            .map_err(|e| RuntimeError::Backend(format!("storage: {e}")))?
            .ok_or_else(|| RuntimeError::Backend("replica not found".into()))?;

        self.store.set_replica_state(replica_id, ReplicaState::Draining).await?;

        self.docker
            .stop_container(
                &container_id,
                Some(StopContainerOptions {
                    t: DRAIN_TIMEOUT_SECS as i64,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| RuntimeError::Backend(format!("stop container: {e}")))?;

        self.stop_replica(replica_id).await
    }

    async fn stop_replica(&self, replica_id: Uuid) -> Result<(), RuntimeError> {
        let container_id = self.store
            .get_replica_backend_instance_id(replica_id)
            .await
            .map_err(|e| RuntimeError::Backend(format!("storage: {e}")))?;

        if let Some(cid) = container_id {
            let _ = self
                .docker
                .stop_container(&cid, Some(StopContainerOptions { t: 0, ..Default::default() }))
                .await;
            let _ = self
                .docker
                .remove_container(
                    &cid,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await;
        }

        self.store.set_replica_state(replica_id, ReplicaState::Stopped).await?;
        tracing::info!(replica_id = %replica_id, "Docker replica stopped");
        Ok(())
    }

    async fn health(&self, replica_id: Uuid) -> Result<HealthStatus, RuntimeError> {
        let container_id = self.store
            .get_replica_backend_instance_id(replica_id)
            .await
            .map_err(|e| RuntimeError::Backend(format!("storage: {e}")))?;

        let Some(cid) = container_id else {
            return Ok(HealthStatus {
                healthy: false,
                message: Some("replica not found".into()),
            });
        };

        let inspect = self
            .docker
            .inspect_container(&cid, None::<InspectContainerOptions>)
            .await
            .map_err(|e| RuntimeError::Backend(format!("inspect: {e}")))?;

        use bollard::models::ContainerStateStatusEnum;
        let healthy = inspect
            .state
            .as_ref()
            .and_then(|s| s.status)
            .map_or(false, |s| s == ContainerStateStatusEnum::RUNNING);
        let message = inspect
            .state
            .as_ref()
            .and_then(|s| s.status.map(|x| x.to_string()));

        Ok(HealthStatus { healthy, message })
    }

    async fn stats(&self, replica_id: Uuid) -> Result<ReplicaStats, RuntimeError> {
        let container_id = self.store
            .get_replica_backend_instance_id(replica_id)
            .await
            .map_err(|e| RuntimeError::Backend(format!("storage: {e}")))?;

        let Some(cid) = container_id else {
            return Ok(ReplicaStats {
                current_load: 0,
                memory_bytes: None,
            });
        };

        let current_load = self.store
            .get_replica_current_load(replica_id)
            .await
            .unwrap_or(0) as u32;

        let memory_bytes = self
            .docker
            .stats(&cid, None)
            .next()
            .await
            .and_then(|r| r.ok())
            .and_then(|s| s.memory_stats.usage);

        Ok(ReplicaStats {
            current_load,
            memory_bytes,
        })
    }
}
