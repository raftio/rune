pub mod a2a;
pub mod canvas;
pub mod channels;
pub mod invoke;
pub mod sessions;
pub mod health;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use rune_env::PlatformEnv;
use rune_storage::RuneStore;

use rune_tools::browser::BrowserManager;
use rune_tools::process::ProcessManager;

struct SharedManagers {
    process_manager: Arc<ProcessManager>,
    browser_manager: Arc<BrowserManager>,
}

static SHARED: OnceLock<SharedManagers> = OnceLock::new();

/// Load an agent's ExecutionPlan from the packages directory, falling back to a stub.
pub fn resolve_plan(agent_name: &str, agent_packages_dir: Option<&str>) -> rune_runtime::ExecutionPlan {
    if let Some(base_str) = agent_packages_dir {
        let base = PathBuf::from(base_str);
        let candidates = [
            base.join(agent_name),
            base.join(format!("rune-{agent_name}")),
        ];
        for agent_dir in &candidates {
            if agent_dir.join("Runefile").exists() {
                match rune_runtime::ExecutionPlan::from_dir(agent_dir) {
                    Ok(plan) => {
                        tracing::info!(
                            agent = agent_name,
                            tools = plan.tools.len(),
                            "loaded agent package from {}",
                            agent_dir.display()
                        );
                        return plan;
                    }
                    Err(e) => {
                        tracing::warn!(
                            agent = agent_name,
                            "failed to load agent package from {}: {e}, falling back to stub",
                            agent_dir.display()
                        );
                    }
                }
            }
        }
    }
    rune_runtime::ExecutionPlan::stub(agent_name)
}

pub fn shared_tool_context(store: &Arc<RuneStore>, env: &Arc<PlatformEnv>) -> Arc<rune_tools::ToolContext> {
    let managers = SHARED.get_or_init(|| SharedManagers {
        process_manager: Arc::new(ProcessManager::new()),
        browser_manager: Arc::new(BrowserManager::new()),
    });

    let agent_ops = rune_runtime::RuntimeAgentOps::new(store.clone(), env.clone());

    Arc::new(rune_tools::ToolContext {
        db: store.pool().clone(),
        workspace_root: env.agent_workspace_dir.as_ref().map(std::path::PathBuf::from),
        process_manager: managers.process_manager.clone(),
        browser_manager: managers.browser_manager.clone(),
        agent_ops: Some(agent_ops),
        env: env.clone(),
    })
}
