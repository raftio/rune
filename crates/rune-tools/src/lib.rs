pub mod defs;
pub mod fs;
pub mod web;
pub mod shell;
pub mod system;
pub mod memory;
pub mod knowledge;
pub mod task;
pub mod schedule;
pub mod process;
pub mod agent;
pub mod media;
pub mod docker;
pub mod browser;
pub mod channel;
pub mod canvas;

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
        // Filesystem
        "file-read" => fs::file_read(&input, ctx).await,
        "file-write" => fs::file_write(&input, ctx).await,
        "file-list" => fs::file_list(&input, ctx).await,
        "apply-patch" => fs::apply_patch(&input, ctx).await,
        // Web
        "web-search" => web::web_search(&input).await,
        "web-fetch" => web::web_fetch(&input).await,
        // Shell
        "shell-exec" => shell::shell_exec(&input).await,
        // System
        "system-time" => Ok(system::system_time()),
        "location-get" => system::location_get().await,
        // Memory
        "memory-store" => memory::store(&input, ctx).await,
        "memory-recall" => memory::recall(&input, ctx).await,
        "memory-list" => memory::list(&input, ctx).await,
        // Knowledge graph
        "knowledge-add-entity" => knowledge::add_entity(&input, ctx).await,
        "knowledge-add-relation" => knowledge::add_relation(&input, ctx).await,
        "knowledge-query" => knowledge::query(&input, ctx).await,
        // Task queue
        "task-post" => task::post(&input, ctx).await,
        "task-claim" => task::claim(&input, ctx).await,
        "task-complete" => task::complete(&input, ctx).await,
        "task-list" => task::list(&input, ctx).await,
        "event-publish" => task::event_publish(&input, ctx).await,
        // Scheduling
        "schedule-create" => schedule::create(&input, ctx).await,
        "schedule-list" => schedule::list(ctx).await,
        "schedule-delete" => schedule::delete(&input, ctx).await,
        "cron-create" => schedule::cron_create(&input, ctx).await,
        "cron-list" => schedule::cron_list(ctx).await,
        "cron-cancel" => schedule::cron_cancel(&input, ctx).await,
        // Process management
        "process-start" => process::start(&input, ctx).await,
        "process-poll" => process::poll(&input, ctx).await,
        "process-write" => process::write(&input, ctx).await,
        "process-kill" => process::kill(&input, ctx).await,
        "process-list" => process::list(ctx).await,
        // Media
        "image-analyze" => media::image_analyze(&input, ctx).await,
        "image-generate" => media::image_generate(&input, ctx).await,
        "media-describe" => media::media_describe(&input, ctx).await,
        "media-transcribe" => media::media_transcribe(&input, ctx).await,
        "text-to-speech" => media::text_to_speech(&input, ctx).await,
        "speech-to-text" => media::speech_to_text(&input, ctx).await,
        // Agent
        "agent-send" => agent::agent_send(&input, ctx).await,
        "agent-list" => agent::agent_list(ctx).await,
        "agent-find" => agent::agent_find(&input, ctx).await,
        "agent-spawn" => agent::agent_spawn(&input, ctx).await,
        "agent-kill" => agent::agent_kill(&input, ctx).await,
        "a2a-discover" => agent::a2a_discover(&input).await,
        "a2a-send" => agent::a2a_send(&input).await,
        // Docker
        "docker-exec" => docker::docker_exec(&input).await,
        // Browser
        "browser-navigate" => browser::navigate(&input, ctx).await,
        "browser-click" => browser::click(&input, ctx).await,
        "browser-type" => browser::type_text(&input, ctx).await,
        "browser-screenshot" => browser::screenshot(ctx).await,
        "browser-read-page" => browser::read_page(ctx).await,
        "browser-close" => browser::close(ctx).await,
        "browser-scroll" => browser::scroll(&input, ctx).await,
        "browser-wait" => browser::wait(&input, ctx).await,
        "browser-run-js" => browser::run_js(&input, ctx).await,
        "browser-back" => browser::back(ctx).await,
        // Channel
        "channel-send" => channel::channel_send(&input, &ctx.env).await,
        // Canvas
        "canvas-present" => canvas::canvas_present(&input, ctx).await,

        other => Err(format!("unknown builtin tool: rune@{other}")),
    }
}
