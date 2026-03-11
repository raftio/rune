use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rune_storage::{RuneStore, RuntimeStore};
use uuid::Uuid;

use rune_network::NetworkRegistry;
use rune_spec::{ToolDescriptor, ToolRuntime};

use crate::error::RuntimeError;
use crate::metrics;
use super::policy::{audit_policy_decision, PolicyDecision, PolicyEngine};
use super::process_runner;
use super::wasm_runner::WasmToolRunner;

const DEFAULT_MAX_AGENT_DEPTH: u32 = 5;

pub struct ToolDispatcher {
    tools: Vec<ToolDescriptor>,
    agent_dir: PathBuf,
    call_depth: u32,
    tool_ctx: Option<Arc<rune_tools::ToolContext>>,
    wasm_runner: Option<Arc<WasmToolRunner>>,
    policy: Option<PolicyEngine>,
    caller_networks: Vec<String>,
}

impl ToolDispatcher {
    pub fn new(tools: Vec<ToolDescriptor>, agent_dir: PathBuf) -> Self {
        Self {
            tools,
            agent_dir,
            call_depth: 0,
            tool_ctx: None,
            wasm_runner: None,
            policy: None,
            caller_networks: vec!["bridge".into()],
        }
    }

    pub fn with_depth(tools: Vec<ToolDescriptor>, agent_dir: PathBuf, depth: u32) -> Self {
        Self {
            tools,
            agent_dir,
            call_depth: depth,
            tool_ctx: None,
            wasm_runner: None,
            policy: None,
            caller_networks: vec!["bridge".into()],
        }
    }

    pub fn with_tool_context(mut self, ctx: Arc<rune_tools::ToolContext>) -> Self {
        self.tool_ctx = Some(ctx);
        self
    }

    pub fn with_wasm_runner(mut self, runner: Arc<WasmToolRunner>) -> Self {
        self.wasm_runner = Some(runner);
        self
    }

    pub fn with_policy(mut self, policy: PolicyEngine) -> Self {
        self.policy = Some(policy);
        self
    }

    pub fn with_caller_networks(mut self, networks: Vec<String>) -> Self {
        self.caller_networks = networks;
        self
    }

    pub fn find(&self, name: &str) -> Option<&ToolDescriptor> {
        self.tools.iter().find(|t| t.name == name)
    }

    fn find_any(&self, name: &str) -> Option<&ToolDescriptor> {
        self.find(name).or_else(|| {
            let canonical = rune_tools::from_wire_name(name);
            self.tools.iter().find(|t| t.name == canonical)
        })
    }

    pub async fn dispatch(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        request_id: Uuid,
        store: &RuneStore,
    ) -> Result<serde_json::Value, RuntimeError> {
        let canonical = rune_tools::from_wire_name(tool_name);
        let _memory_key = input.get("key").cloned();

        let descriptor = self
            .find_any(tool_name)
            .ok_or_else(|| RuntimeError::Engine(format!("tool '{}' not found in plan", tool_name)))?
            .clone();

        let invocation_id = Uuid::new_v4().to_string();
        let input_str = serde_json::to_string(&input)
            .map_err(|e| RuntimeError::Engine(e.to_string()))?;
        let runtime_str = format!("{:?}", descriptor.runtime).to_lowercase();

        store
            .insert_tool_invocation(&invocation_id, request_id, &canonical, &runtime_str, &input_str)
            .await?;

        if let Some(ref policy) = self.policy {
            let decision = policy.check_tool(&canonical);
            let _ = audit_policy_decision(
                store,
                &decision,
                "tool",
                &canonical,
                Some(&request_id.to_string()),
            )
            .await;
            if let PolicyDecision::Deny { reason } = decision {
                metrics::increment_policy_denials();
                let err = RuntimeError::Engine(format!("policy denied: {reason}"));
                let _ = store
                    .update_tool_invocation_failed(&invocation_id, &err.to_string())
                    .await;
                return Err(err);
            }
        }

        let start = std::time::Instant::now();
        let result = match descriptor.runtime {
            ToolRuntime::Builtin => {
                if let Some(ctx) = &self.tool_ctx {
                    rune_tools::execute(&canonical, input, ctx)
                        .await
                        .map_err(|e| RuntimeError::ToolExecution(e))
                } else {
                    Err(RuntimeError::ToolExecution(
                        "ToolContext not configured for builtin tools".into(),
                    ))
                }
            }
            ToolRuntime::Process => {
                let env_map = self.tool_ctx.as_ref().map(|ctx| {
                    rune_env::AgentEnv::resolve(&ctx.env, None).to_map()
                });
                process_runner::run_process(&descriptor, &self.agent_dir, input, env_map.as_ref()).await
            }
            ToolRuntime::Wasm => {
                let runner = match self.wasm_runner.clone() {
                    Some(r) => r,
                    None => Arc::new(
                        WasmToolRunner::new()
                            .map_err(|e| RuntimeError::ToolExecution(format!("WasmToolRunner init failed: {e}")))?
                    ),
                };
                runner.run(&descriptor, &self.agent_dir, input).await
            }
            ToolRuntime::Container => {
                tracing::warn!(tool = tool_name, "Container tool runtime not yet implemented, returning stub");
                Ok(serde_json::json!({ "stub": true, "tool": tool_name }))
            }
            ToolRuntime::Agent => {
                self.dispatch_agent(&descriptor, input, request_id, store).await
            }
            ToolRuntime::Mcp => {
                self.dispatch_mcp(&descriptor, input).await
            }
        };

        let status = if result.is_ok() { "ok" } else { "error" };
        metrics::record_tool_call_duration(&canonical, status, start.elapsed().as_secs_f64());

        match &result {
            Ok(output) => {
                let output_str = serde_json::to_string(output)
                    .map_err(|e| RuntimeError::Engine(e.to_string()))?;
                store
                    .update_tool_invocation_completed(&invocation_id, &output_str)
                    .await?;
            }
            Err(e) => {
                let _ = store
                    .update_tool_invocation_failed(&invocation_id, &e.to_string())
                    .await;
            }
        }

        result
    }

    async fn dispatch_agent(
        &self,
        descriptor: &ToolDescriptor,
        input: serde_json::Value,
        request_id: Uuid,
        store: &RuneStore,
    ) -> Result<serde_json::Value, RuntimeError> {
        let agent_ref = descriptor.agent_ref.as_deref().ok_or_else(|| {
            RuntimeError::Engine(format!(
                "agent tool '{}' missing agent_ref field",
                descriptor.name
            ))
        })?;

        let max_depth = descriptor.max_depth.unwrap_or(DEFAULT_MAX_AGENT_DEPTH);
        if self.call_depth >= max_depth {
            return Err(RuntimeError::Engine(format!(
                "agent call depth {} exceeds max_depth {}",
                self.call_depth, max_depth
            )));
        }

        let endpoint = if let Some(callee_name) = agent_ref.strip_prefix("local://") {
            let registry = NetworkRegistry::new(store.clone());

            registry
                .check_access(&descriptor.name, &self.caller_networks, callee_name)
                .await
                .map_err(|e| RuntimeError::Engine(e.to_string()))?;

            match registry.direct_replica_endpoint(callee_name).await {
                Ok(Some(base)) => {
                    tracing::debug!(
                        callee = callee_name,
                        base = %base,
                        "A2A: direct replica routing (gateway bypassed)"
                    );
                    format!("{}/a2a/{}", base.trim_end_matches('/'), callee_name)
                }
                _ => {
                    let gw = self.tool_ctx
                        .as_ref()
                        .map(|ctx| ctx.env.gateway_base_url.clone())
                        .unwrap_or_else(|| {
                            tracing::warn!("no gateway_base_url configured, falling back to http://localhost:3000");
                            "http://localhost:3000".into()
                        });
                    format!("{}/a2a/{}", gw.trim_end_matches('/'), callee_name)
                }
            }
        } else {
            agent_ref.to_string()
        };

        let call_id = Uuid::new_v4().to_string();
        let started = std::time::Instant::now();

        let _ = store
            .insert_agent_call(
                &call_id,
                &request_id.to_string(),
                "self",
                &descriptor.name,
                &endpoint,
                self.call_depth as i64 + 1,
            )
            .await;

        let timeout = Duration::from_millis(descriptor.timeout_ms);
        let a2a_client = rune_a2a::A2aClient::new(timeout);

        let message = rune_a2a::Message {
            message_id: Uuid::new_v4().to_string(),
            role: rune_a2a::Role::User,
            parts: vec![rune_a2a::Part::data(input)],
            metadata: Some(serde_json::json!({
                "rune_call_depth": self.call_depth + 1,
                "rune_parent_request": request_id.to_string(),
            })),
            ..Default::default()
        };

        let result = a2a_client
            .send_message_blocking(&endpoint, message)
            .await
            .map_err(|e| RuntimeError::ToolExecution(format!("A2A agent call failed: {e}")))?;

        let elapsed_ms = started.elapsed().as_millis() as i64;

        let output = match &result {
            rune_a2a::SendMessageResult::Task(task) => {
                let _ = store
                    .update_agent_call_callee_task(&call_id, &task.id)
                    .await;

                if task.status.state == rune_a2a::TaskState::Failed {
                    return Err(RuntimeError::ToolExecution(format!(
                        "remote agent '{}' task failed",
                        descriptor.name
                    )));
                }

                task.data_output()
                    .or_else(|| task.text_output().map(|t| serde_json::json!({ "text": t })))
                    .unwrap_or(serde_json::json!({ "completed": true }))
            }
            rune_a2a::SendMessageResult::Message(msg) => {
                extract_message_output(msg)
            }
        };

        let _ = store
            .update_agent_call_completed(&call_id, elapsed_ms)
            .await;

        Ok(output)
    }

    async fn dispatch_mcp(
        &self,
        descriptor: &ToolDescriptor,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        let server_name = descriptor.mcp_server.as_deref().ok_or_else(|| {
            RuntimeError::ToolExecution(format!(
                "MCP tool '{}' missing mcp_server field",
                descriptor.name
            ))
        })?;

        // Resolve MCP server URL from env: RUNE_MCP_{SERVER_NAME}_URL (uppercased, hyphens→underscores).
        let env_key = format!(
            "RUNE_MCP_{}_URL",
            server_name.to_uppercase().replace('-', "_")
        );
        let url = std::env::var(&env_key).map_err(|_| {
            RuntimeError::ToolExecution(format!(
                "MCP server '{}' URL not configured (set {})",
                server_name, env_key
            ))
        })?;

        // Strip the `{server_name}/` prefix from the tool name to get the MCP tool name.
        let mcp_tool_name = descriptor.name
            .strip_prefix(&format!("{server_name}/"))
            .unwrap_or(&descriptor.name);

        let client = rune_mcp::McpClient::new(url, Default::default());
        let result = client
            .call_tool(mcp_tool_name, input)
            .await
            .map_err(|e| RuntimeError::ToolExecution(format!("MCP call failed: {e}")))?;

        Ok(result.to_value())
    }
}


fn extract_message_output(msg: &rune_a2a::Message) -> serde_json::Value {
    for part in &msg.parts {
        match part {
            rune_a2a::Part::Data { data, .. } => return data.clone(),
            rune_a2a::Part::Text { text, .. } => {
                return serde_json::json!({ "text": text });
            }
            _ => {}
        }
    }
    serde_json::json!({ "message": "empty response" })
}
