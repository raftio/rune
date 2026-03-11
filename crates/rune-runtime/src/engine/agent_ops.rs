use std::sync::Arc;
use std::time::Duration;

use rune_env::PlatformEnv;
use rune_storage::RuneStore;
use uuid::Uuid;

use rune_tools::AgentOps;

pub struct RuntimeAgentOps {
    store: Arc<RuneStore>,
    gateway_base: String,
}

impl RuntimeAgentOps {
    pub fn new(store: Arc<RuneStore>, env: Arc<PlatformEnv>) -> Arc<Self> {
        Arc::new(Self {
            store,
            gateway_base: env.gateway_base_url.clone(),
        })
    }
}

#[async_trait::async_trait]
impl AgentOps for RuntimeAgentOps {
    async fn send_message(&self, agent: &str, message: &str) -> Result<serde_json::Value, String> {
        let endpoint = format!(
            "{}/a2a/{}",
            self.gateway_base.trim_end_matches('/'),
            agent
        );

        let client = rune_a2a::A2aClient::new(Duration::from_secs(60));
        let msg = rune_a2a::Message {
            message_id: Uuid::new_v4().to_string(),
            role: rune_a2a::Role::User,
            parts: vec![rune_a2a::Part::text(message)],
            ..Default::default()
        };

        let result = client
            .send_message_blocking(&endpoint, msg)
            .await
            .map_err(|e| format!("agent-send failed: {e}"))?;

        match result {
            rune_a2a::SendMessageResult::Task(task) => {
                let text = task.text_output().unwrap_or_default();
                let data = task.data_output();
                Ok(serde_json::json!({
                    "task_id": task.id,
                    "state": format!("{:?}", task.status.state),
                    "text": text,
                    "data": data,
                }))
            }
            rune_a2a::SendMessageResult::Message(msg) => {
                let text: Vec<&str> = msg
                    .parts
                    .iter()
                    .filter_map(|p| {
                        if let rune_a2a::Part::Text { text, .. } = p {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();
                Ok(serde_json::json!({
                    "message_id": msg.message_id,
                    "text": text.join("\n"),
                }))
            }
        }
    }

    async fn list_agents(&self) -> Result<serde_json::Value, String> {
        let rows = self.store.list_agents().await.map_err(|e| format!("Failed to list agents: {e}"))?;

        let agents: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "name": r.agent_name,
                    "status": r.status,
                    "project": r.project_ref,
                    "replicas": r.desired_replicas,
                })
            })
            .collect();

        Ok(serde_json::json!({ "agents": agents, "count": agents.len() }))
    }

    async fn find_agent(&self, query: &str) -> Result<serde_json::Value, String> {
        let rows = self.store.find_agents(query).await.map_err(|e| format!("Failed to find agents: {e}"))?;

        let agents: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|r| {
                serde_json::json!({ "id": r.id, "name": r.agent_name, "status": r.status })
            })
            .collect();

        Ok(serde_json::json!({ "results": agents, "query": query }))
    }

    async fn spawn_agent(&self, manifest: &str) -> Result<serde_json::Value, String> {
        let spec: serde_json::Value =
            serde_yaml::from_str(manifest).map_err(|e| format!("Invalid manifest: {e}"))?;

        let agent_name = spec["name"]
            .as_str()
            .unwrap_or("unnamed-agent")
            .to_string();

        let (_version_id, deployment_id) = self.store
            .spawn_agent(&agent_name)
            .await
            .map_err(|e| format!("Failed to spawn agent: {e}"))?;

        Ok(serde_json::json!({
            "deployment_id": deployment_id,
            "agent_name": agent_name,
            "status": "active",
        }))
    }

    async fn kill_agent(&self, agent_id: &str) -> Result<serde_json::Value, String> {
        let affected = self.store
            .kill_agent(agent_id)
            .await
            .map_err(|e| format!("Failed to kill agent: {e}"))?;

        if affected == 0 {
            return Err(format!("Agent deployment '{agent_id}' not found"));
        }

        Ok(serde_json::json!({
            "killed": true,
            "agent_id": agent_id,
        }))
    }
}
