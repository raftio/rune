pub mod defs;
pub mod memory;
pub mod process;
pub mod browser;

use std::path::PathBuf;
use std::sync::Arc;

use rune_env::PlatformEnv;

pub const BUILTIN_PREFIX: &str = "rune@";
pub const WIRE_PREFIX: &str = "rune__";

#[async_trait::async_trait]
pub trait AgentOps: Send + Sync {
    async fn send_message(&self, agent: &str, message: &str) -> Result<serde_json::Value, String>;
    async fn list_agents(&self) -> Result<serde_json::Value, String>;
    async fn find_agent(&self, query: &str) -> Result<serde_json::Value, String>;
    async fn spawn_agent(&self, manifest: &str) -> Result<serde_json::Value, String>;
    async fn kill_agent(&self, agent_id: &str) -> Result<serde_json::Value, String>;
}

/// Shared context passed to tool implementations that need external state.
pub struct ToolContext {
    pub db: sqlx::SqlitePool,
    pub workspace_root: Option<PathBuf>,
    pub process_manager: Arc<process::ProcessManager>,
    pub browser_manager: Arc<browser::BrowserManager>,
    pub agent_ops: Option<Arc<dyn AgentOps>>,
    pub env: Arc<PlatformEnv>,
    /// Name of the currently running agent, used as default for schedule ownership.
    pub agent_name: Option<String>,
}

/// A built-in tool definition sent to LLMs.
#[derive(Debug, Clone)]
pub struct BuiltinToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: serde_json::Value,
}

pub fn is_builtin(name: &str) -> bool {
    name.starts_with(BUILTIN_PREFIX) || name.starts_with(WIRE_PREFIX)
}

pub fn to_wire_name(name: &str) -> String {
    if let Some(suffix) = name.strip_prefix(BUILTIN_PREFIX) {
        format!("{WIRE_PREFIX}{suffix}")
    } else {
        name.to_string()
    }
}

pub fn from_wire_name(name: &str) -> String {
    if let Some(suffix) = name.strip_prefix(WIRE_PREFIX) {
        format!("{BUILTIN_PREFIX}{suffix}")
    } else {
        name.to_string()
    }
}

pub fn all_definitions() -> Vec<BuiltinToolDef> {
    defs::all()
}

pub fn find_definition(name: &str) -> Option<BuiltinToolDef> {
    let canonical = from_wire_name(name);
    defs::all().into_iter().find(|d| d.name == canonical)
}

pub async fn execute(
    name: &str,
    input: serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let canonical = from_wire_name(name);
    let suffix = canonical
        .strip_prefix(BUILTIN_PREFIX)
        .ok_or_else(|| format!("not a builtin tool: {name}"))?;

    match suffix {
        // Memory
        "memory-store" => memory::store(&input, ctx).await,
        "memory-recall" => memory::recall(&input, ctx).await,
        "memory-list" => memory::list(&input, ctx).await,

        other => Err(format!("unknown builtin tool: rune@{other}")),
    }
}
