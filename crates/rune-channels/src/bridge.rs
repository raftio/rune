use crate::config::{ChannelOverrides, DmPolicy, GroupPolicy, OutputFormat};
use crate::formatter;
use crate::router::AgentRouter;
use crate::types::{
    default_phase_emoji, AgentPhase, ChannelAdapter, ChannelContent, ChannelMessage, ChannelUser,
    LifecycleReaction,
};
use async_trait::async_trait;
use dashmap::DashMap;
use futures::StreamExt;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Kernel operations needed by channel adapters.
///
/// Implemented by rune-gateway/runtime to connect channels to the agent runtime.
#[async_trait]
pub trait ChannelBridgeHandle: Send + Sync {
    /// Send a message to an agent and get the text response.
    async fn send_message(&self, agent_id: Uuid, message: &str) -> Result<String, String>;

    /// Find an agent by name, returning its ID.
    async fn find_agent_by_name(&self, name: &str) -> Result<Option<Uuid>, String>;

    /// List running agents as (id, name) pairs.
    async fn list_agents(&self) -> Result<Vec<(Uuid, String)>, String>;

    /// Spawn an agent by manifest name, returning its ID.
    async fn spawn_agent_by_name(&self, manifest_name: &str) -> Result<Uuid, String>;

    /// Return uptime info string.
    async fn uptime_info(&self) -> String {
        let agents = self.list_agents().await.unwrap_or_default();
        format!("{} agent(s) running", agents.len())
    }

    /// Authorize a channel user for an action.
    async fn authorize_channel_user(
        &self,
        _channel_type: &str,
        _platform_id: &str,
        _action: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    /// Get per-channel overrides for a given channel type.
    async fn channel_overrides(&self, _channel_type: &str) -> Option<ChannelOverrides> {
        None
    }

    /// Record a delivery result for tracking.
    async fn record_delivery(
        &self,
        _agent_id: Uuid,
        _channel: &str,
        _recipient: &str,
        _success: bool,
        _error: Option<&str>,
    ) {
    }

    /// Reset an agent's session.
    async fn reset_session(&self, _agent_id: Uuid) -> Result<String, String> {
        Err("Not implemented".to_string())
    }
}

/// Per-channel rate limiter tracking message timestamps per user.
#[derive(Debug, Clone, Default)]
pub struct ChannelRateLimiter {
    buckets: Arc<DashMap<String, Vec<Instant>>>,
}

impl ChannelRateLimiter {
    /// Check if a user is rate-limited. Returns `Ok(())` if allowed, `Err(msg)` if blocked.
    pub fn check(
        &self,
        channel_type: &str,
        platform_id: &str,
        max_per_minute: u32,
    ) -> Result<(), String> {
        if max_per_minute == 0 {
            return Ok(());
        }

        let key = format!("{channel_type}:{platform_id}");
        let now = Instant::now();
        let window = std::time::Duration::from_secs(60);

        let mut entry = self.buckets.entry(key).or_default();
        entry.retain(|&ts| now.duration_since(ts) < window);

        if entry.len() >= max_per_minute as usize {
            return Err(format!(
                "Rate limit exceeded ({max_per_minute} messages/minute). Please wait."
            ));
        }

        entry.push(now);
        Ok(())
    }
}

/// Owns all running channel adapters and dispatches messages to agents.
pub struct BridgeManager {
    handle: Arc<dyn ChannelBridgeHandle>,
    router: Arc<AgentRouter>,
    rate_limiter: ChannelRateLimiter,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    adapters: Vec<Arc<dyn ChannelAdapter>>,
}

impl BridgeManager {
    pub fn new(handle: Arc<dyn ChannelBridgeHandle>, router: Arc<AgentRouter>) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            handle,
            router,
            rate_limiter: ChannelRateLimiter::default(),
            shutdown_tx,
            shutdown_rx,
            tasks: Vec::new(),
            adapters: Vec::new(),
        }
    }

    /// Start an adapter: subscribe to its message stream and spawn a dispatch task.
    pub async fn start_adapter(
        &mut self,
        adapter: Arc<dyn ChannelAdapter>,
    ) -> Result<(), crate::error::ChannelError> {
        let stream = adapter.start().await?;
        let handle = self.handle.clone();
        let router = self.router.clone();
        let rate_limiter = self.rate_limiter.clone();
        let adapter_clone = adapter.clone();
        let mut shutdown = self.shutdown_rx.clone();

        let task = tokio::spawn(async move {
            let mut stream = std::pin::pin!(stream);
            loop {
                tokio::select! {
                    msg = stream.next() => {
                        match msg {
                            Some(message) => {
                                dispatch_message(
                                    &message,
                                    &handle,
                                    &router,
                                    adapter_clone.as_ref(),
                                    &rate_limiter,
                                ).await;
                            }
                            None => {
                                info!("Channel adapter {} stream ended", adapter_clone.name());
                                break;
                            }
                        }
                    }
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() {
                            info!("Shutting down channel adapter {}", adapter_clone.name());
                            break;
                        }
                    }
                }
            }
        });

        self.adapters.push(adapter);
        self.tasks.push(task);
        Ok(())
    }

    /// Get status of all registered adapters.
    pub fn adapter_statuses(&self) -> Vec<(String, crate::types::ChannelStatus)> {
        self.adapters
            .iter()
            .map(|a| (a.name().to_string(), a.status()))
            .collect()
    }

    /// Stop all adapters and wait for dispatch tasks to finish.
    pub async fn stop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        for adapter in &self.adapters {
            if let Err(e) = adapter.stop().await {
                warn!(adapter = adapter.name(), error = %e, "Failed to stop adapter");
            }
        }
        for task in self.tasks.drain(..) {
            let _ = task.await;
        }
    }

    /// Get a reference to the router.
    pub fn router(&self) -> &Arc<AgentRouter> {
        &self.router
    }
}

async fn send_response(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    text: String,
    thread_id: Option<&str>,
    output_format: OutputFormat,
) {
    let formatted = formatter::format_for_channel(&text, output_format);
    let content = ChannelContent::Text(formatted);

    let result = if let Some(tid) = thread_id {
        adapter.send_in_thread(user, content, tid).await
    } else {
        adapter.send(user, content).await
    };

    if let Err(e) = result {
        error!("Failed to send response: {e}");
    }
}

async fn send_lifecycle_reaction(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    message_id: &str,
    phase: AgentPhase,
) {
    let reaction = LifecycleReaction {
        emoji: default_phase_emoji(&phase).to_string(),
        phase,
        remove_previous: true,
    };
    let _ = adapter.send_reaction(user, message_id, &reaction).await;
}

/// Dispatch a single incoming message.
async fn dispatch_message(
    message: &ChannelMessage,
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
    adapter: &dyn ChannelAdapter,
    rate_limiter: &ChannelRateLimiter,
) {
    let ct_str = message.channel.as_str();

    let overrides = handle.channel_overrides(ct_str).await;
    let channel_default_format = match ct_str {
        "telegram" => OutputFormat::TelegramHtml,
        "slack" => OutputFormat::SlackMrkdwn,
        _ => OutputFormat::Markdown,
    };
    let output_format = overrides
        .as_ref()
        .and_then(|o| o.output_format)
        .unwrap_or(channel_default_format);
    let threading_enabled = overrides.as_ref().map(|o| o.threading).unwrap_or(false);
    let thread_id = if threading_enabled {
        message.thread_id.as_deref()
    } else {
        None
    };

    // DM/Group policy check
    if let Some(ref ov) = overrides {
        if message.is_group {
            match ov.group_policy {
                GroupPolicy::Ignore => {
                    debug!("Ignoring group message on {ct_str} (group_policy=ignore)");
                    return;
                }
                GroupPolicy::CommandsOnly => {
                    let is_command = matches!(&message.content, ChannelContent::Command { .. })
                        || matches!(&message.content, ChannelContent::Text(t) if t.starts_with('/'));
                    if !is_command {
                        debug!("Ignoring non-command group message on {ct_str}");
                        return;
                    }
                }
                GroupPolicy::MentionOnly => {
                    let was_mentioned = message
                        .metadata
                        .get("was_mentioned")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let is_command = matches!(&message.content, ChannelContent::Command { .. });
                    if !was_mentioned && !is_command {
                        debug!("Ignoring group message on {ct_str} (not mentioned)");
                        return;
                    }
                }
                GroupPolicy::All => {}
            }
        } else {
            match ov.dm_policy {
                DmPolicy::Ignore => {
                    debug!("Ignoring DM on {ct_str}");
                    return;
                }
                DmPolicy::AllowedOnly => {}
                DmPolicy::Respond => {}
            }
        }
    }

    // Rate limiting
    if let Some(ref ov) = overrides {
        if ov.rate_limit_per_user > 0 {
            if let Err(msg) =
                rate_limiter.check(ct_str, &message.sender.platform_id, ov.rate_limit_per_user)
            {
                send_response(adapter, &message.sender, msg, thread_id, output_format).await;
                return;
            }
        }
    }

    // Handle commands
    if let ChannelContent::Command { ref name, ref args } = message.content {
        let result = handle_command(name, args, handle, router, &message.sender).await;
        send_response(adapter, &message.sender, result, thread_id, output_format).await;
        return;
    }

    let text = match &message.content {
        ChannelContent::Text(t) => t.clone(),
        ChannelContent::Command { .. } => {
            // Already handled above and returned early; this branch is unreachable
            // but we avoid panic!() in production by returning gracefully.
            tracing::warn!("Unexpected Command content reached text dispatch — skipping");
            return;
        }
        ChannelContent::Image { url, caption } => match caption {
            Some(c) => format!("[User sent a photo: {url}]\nCaption: {c}"),
            None => format!("[User sent a photo: {url}]"),
        },
        ChannelContent::File { url, filename } => {
            format!("[User sent a file ({filename}): {url}]")
        }
        ChannelContent::Voice {
            url,
            duration_seconds,
        } => {
            format!("[User sent a voice message ({duration_seconds}s): {url}]")
        }
        ChannelContent::Location { lat, lon } => {
            format!("[User shared location: {lat}, {lon}]")
        }
    };

    // Check text-embedded slash commands
    if text.starts_with('/') {
        let parts: Vec<&str> = text.splitn(2, ' ').collect();
        let cmd = &parts[0][1..];
        let args: Vec<String> = if parts.len() > 1 {
            parts[1].split_whitespace().map(String::from).collect()
        } else {
            vec![]
        };

        if matches!(
            cmd,
            "start" | "help" | "agents" | "agent" | "status" | "new" | "stop"
        ) {
            let result = handle_command(cmd, &args, handle, router, &message.sender).await;
            send_response(adapter, &message.sender, result, thread_id, output_format).await;
            return;
        }
    }

    // RBAC check
    if let Err(denied) = handle
        .authorize_channel_user(ct_str, &message.sender.platform_id, "chat")
        .await
    {
        send_response(
            adapter,
            &message.sender,
            format!("Access denied: {denied}"),
            thread_id,
            output_format,
        )
        .await;
        return;
    }

    // Broadcast routing
    if router.has_broadcast(&message.sender.platform_id) {
        let targets = router.resolve_broadcast(&message.sender.platform_id);
        if !targets.is_empty() {
            let _ = adapter.send_typing(&message.sender).await;

            let strategy = router.broadcast_strategy();
            let mut responses = Vec::new();

            match strategy {
                crate::config::BroadcastStrategy::Parallel => {
                    let mut handles_vec = Vec::new();
                    for (name, maybe_id) in &targets {
                        if let Some(aid) = maybe_id {
                            let h = handle.clone();
                            let t = text.clone();
                            let aid = *aid;
                            let name = name.clone();
                            handles_vec.push(tokio::spawn(async move {
                                let result = h.send_message(aid, &t).await;
                                (name, result)
                            }));
                        }
                    }
                    for jh in handles_vec {
                        if let Ok((name, result)) = jh.await {
                            match result {
                                Ok(r) => responses.push(format!("[{name}]: {r}")),
                                Err(e) => responses.push(format!("[{name}]: Error: {e}")),
                            }
                        }
                    }
                }
                crate::config::BroadcastStrategy::Sequential => {
                    for (name, maybe_id) in &targets {
                        if let Some(aid) = maybe_id {
                            match handle.send_message(*aid, &text).await {
                                Ok(r) => responses.push(format!("[{name}]: {r}")),
                                Err(e) => responses.push(format!("[{name}]: Error: {e}")),
                            }
                        }
                    }
                }
            }

            let combined = responses.join("\n\n");
            send_response(adapter, &message.sender, combined, thread_id, output_format).await;
            return;
        }
    }

    // Standard single-agent routing
    let agent_id = router.resolve(
        &message.channel,
        &message.sender.platform_id,
        message.sender.rune_user.as_deref(),
    );

    let agent_id = match agent_id {
        Some(id) => id,
        None => {
            send_response(
                adapter,
                &message.sender,
                "No agent configured. Use /agents to see available agents.".to_string(),
                thread_id,
                output_format,
            )
            .await;
            return;
        }
    };

    let _ = adapter.send_typing(&message.sender).await;
    send_lifecycle_reaction(
        adapter,
        &message.sender,
        &message.platform_message_id,
        AgentPhase::Thinking,
    )
    .await;

    let response = handle.send_message(agent_id, &text).await;
    let (phase, reply, success) = match response {
        Ok(r) => (AgentPhase::Done, r, true),
        Err(e) => {
            error!(agent_id = %agent_id, error = %e, "Agent invocation failed");
            (AgentPhase::Error, format!("Error: {e}"), false)
        }
    };

    send_lifecycle_reaction(
        adapter,
        &message.sender,
        &message.platform_message_id,
        phase,
    )
    .await;
    send_response(adapter, &message.sender, reply, thread_id, output_format).await;
    handle
        .record_delivery(
            agent_id,
            ct_str,
            &message.sender.platform_id,
            success,
            if success { None } else { Some("agent error") },
        )
        .await;
}

async fn handle_command(
    name: &str,
    args: &[String],
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
    sender: &ChannelUser,
) -> String {
    match name {
        "start" | "help" => {
            "Available commands:\n\
             /agents — list running agents\n\
             /agent <name> — switch to a specific agent\n\
             /status — show system status\n\
             /new — start a fresh session\n\
             /stop — stop the current agent run"
                .to_string()
        }
        "agents" => match handle.list_agents().await {
            Ok(agents) => {
                if agents.is_empty() {
                    "No agents running.".to_string()
                } else {
                    let lines: Vec<String> = agents
                        .iter()
                        .map(|(id, name)| format!("- {name} ({id})"))
                        .collect();
                    format!("Running agents:\n{}", lines.join("\n"))
                }
            }
            Err(e) => format!("Error listing agents: {e}"),
        },
        "agent" => {
            if args.is_empty() {
                return "Usage: /agent <name>".to_string();
            }
            let name = &args[0];
            match handle.find_agent_by_name(name).await {
                Ok(Some(id)) => {
                    router.set_user_default(sender.platform_id.clone(), id);
                    format!("Switched to agent '{name}'.")
                }
                Ok(None) => format!("Agent '{name}' not found."),
                Err(e) => format!("Error: {e}"),
            }
        }
        "status" => handle.uptime_info().await,
        "new" => {
            let agent_id = router.resolve(
                &crate::types::ChannelType::Custom("cmd".into()),
                &sender.platform_id,
                sender.rune_user.as_deref(),
            );
            match agent_id {
                Some(id) => match handle.reset_session(id).await {
                    Ok(msg) => msg,
                    Err(e) => format!("Error resetting session: {e}"),
                },
                None => "No agent selected. Use /agent <name> first.".to_string(),
            }
        }
        "stop" => "Agent run stopped.".to_string(),
        _ => format!("Unknown command: /{name}. Type /help for available commands."),
    }
}
