use async_trait::async_trait;
use rune_channels::{ChannelBridgeHandle, ChannelOverrides};
use rune_env::PlatformEnv;
use rune_runtime::{ExecutionPlan, LlmClient, Planner, SessionManager, StubPlanner, ToolDispatcher};
use rune_storage::{RuneStore, RuntimeStore};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

const AGENT_EXECUTION_TIMEOUT: Duration = Duration::from_secs(120);

/// Connects channel adapters to the Rune agent runtime.
///
/// Implements `ChannelBridgeHandle` so `BridgeManager` can route
/// inbound channel messages to agents.
pub struct RuntimeChannelBridge {
    store: Arc<RuneStore>,
    env: Arc<PlatformEnv>,
}

impl RuntimeChannelBridge {
    pub fn new(store: Arc<RuneStore>, env: Arc<PlatformEnv>) -> Self {
        Self { store, env }
    }
}

#[async_trait]
impl ChannelBridgeHandle for RuntimeChannelBridge {
    async fn send_message(&self, agent_id: Uuid, message: &str) -> Result<String, String> {
        let sessions = SessionManager::new(self.store.clone());
        let session_id = sessions
            .create(agent_id, None, None)
            .await
            .map_err(|e| e.to_string())?;

        let input = serde_json::json!({ "message": message });

        let store = self.store.clone();
        let agent_id_str = agent_id.to_string();

        let env_overrides = if self.env.agent_packages_dir.as_ref().map_or(true, |s| s.is_empty())
            && self.env.channel_agent.as_ref().map_or(true, |s| s.is_empty())
        {
            let dep_result = self.store.get_agent_and_env_for_deployment(agent_id).await;
            dep_result.ok().flatten().map(|(agent_name, env_json)| {
                let env_map = env_json
                    .as_ref()
                    .and_then(|j| serde_json::from_str::<std::collections::HashMap<String, String>>(j).ok())
                    .unwrap_or_default();
                let dir = env_map
                    .get("AGENT_PACKAGES_DIR")
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .unwrap_or_else(|| ".".to_string());
                let chan = env_map
                    .get("RUNE_CHANNEL_AGENT")
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .unwrap_or(agent_name.clone());
                (dir, chan)
            })
        } else {
            None
        };

        let result = tokio::time::timeout(AGENT_EXECUTION_TIMEOUT, async {
            if let Some(client) = LlmClient::from_platform_env(&self.env).or_else(LlmClient::from_env) {
                let plan = resolve_plan(&agent_id_str, &self.env, env_overrides.as_ref());
                let tool_ctx = crate::routes::shared_tool_context(&store, &self.env, Some(&plan.agent_name));
                let policy = rune_runtime::PolicyEngine::new(
                    plan.toolset.clone().into_iter(),
                    plan.models.clone(),
                );
                let tools = ToolDispatcher::new(plan.tools.clone(), plan.agent_dir.clone())
                    .with_tool_context(tool_ctx)
                    .with_policy(policy);
                let planner = Planner::new(plan);
                let request_id = Uuid::new_v4();
                store
                    .insert_request(request_id, session_id, agent_id, None, &input)
                    .await
                    .map_err(|e| e.to_string())?;
                planner
                    .run(&client, &sessions, session_id, input, &tools, request_id, &store)
                    .await
                    .map_err(|e| e.to_string())
            } else {
                let stub = StubPlanner::new(&agent_id_str);
                stub.run(&sessions, session_id, input)
                    .await
                    .map_err(|e| e.to_string())
            }
        })
        .await
        .map_err(|_| format!("Agent execution timed out after {}s", AGENT_EXECUTION_TIMEOUT.as_secs()))?;

        let output = result?;

        output
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| output.get("text").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .or_else(|| serde_json::to_string(&output).ok())
            .ok_or_else(|| "Empty output".to_string())
    }

    async fn find_agent_by_name(&self, name: &str) -> Result<Option<Uuid>, String> {
        self.store
            .resolve_deployment_for_agent(name)
            .await
            .map_err(|e| e.to_string())
    }

    async fn list_agents(&self) -> Result<Vec<(Uuid, String)>, String> {
        let names = self
            .store
            .list_deployed_agent_names()
            .await
            .map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for name in names {
            if let Ok(Some(id)) = self.store.resolve_deployment_for_agent(&name).await {
                result.push((id, name));
            }
        }
        Ok(result)
    }

    async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<Uuid, String> {
        Err("Spawning agents from channels is not yet supported".to_string())
    }

    async fn channel_overrides(&self, _channel_type: &str) -> Option<ChannelOverrides> {
        None
    }
}

/// Resolve an execution plan for a channel-invoked agent.
///
/// Priority: overrides (from deployment env) > env vars > AGENT_PACKAGES_DIR lookup.
/// overrides: (agent_packages_dir, channel_agent) from deployment env when gateway env is empty.
fn resolve_plan(
    agent_id: &str,
    env: &PlatformEnv,
    overrides: Option<&(String, String)>,
) -> ExecutionPlan {
    let (base_override, chan_override) = overrides
        .map(|(d, c)| (d.as_str(), c.as_str()))
        .unwrap_or(("", ""));
    let base_str = env.agent_packages_dir.as_deref().unwrap_or(base_override);
    let chan_str = env.channel_agent.as_deref().unwrap_or(chan_override);

    if !chan_str.is_empty() {
        let base = if base_str.is_empty() { "." } else { base_str };
        let path_subdir = std::path::PathBuf::from(base).join(chan_str);
        let path_base = std::path::PathBuf::from(base);
        for path in [&path_subdir, &path_base] {
            if path.join("Runefile").exists() {
                if let Ok(plan) = ExecutionPlan::from_dir(path) {
                    return plan;
                }
            }
        }
        return ExecutionPlan::stub(chan_str);
    }

    if !base_str.is_empty() {
        let base = std::path::PathBuf::from(base_str);
        if base.join("Runefile").exists() {
            if let Ok(plan) = ExecutionPlan::from_dir(&base) {
                return plan;
            }
        }
        let read_result = std::fs::read_dir(&base);
        let mut candidates: Vec<_> = read_result
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().join("Runefile").exists())
            .collect();
        candidates.sort_by_key(|e| e.file_name());
        if let Some(entry) = candidates.first() {
            if candidates.len() > 1 {
                tracing::warn!(
                    agent = ?entry.file_name(),
                    total = candidates.len(),
                    "Multiple agents in AGENT_PACKAGES_DIR; picking first alphabetically. \
                     Set RUNE_CHANNEL_AGENT for deterministic selection."
                );
            }
            if let Ok(plan) = ExecutionPlan::from_dir(&entry.path()) {
                return plan;
            }
        }
    }

    ExecutionPlan::stub(agent_id)
}
