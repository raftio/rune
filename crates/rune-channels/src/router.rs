use crate::config::{AgentBinding, BroadcastConfig, BroadcastStrategy};
use crate::types::ChannelType;
use dashmap::DashMap;
use std::sync::Mutex;
use tracing::warn;
use uuid::Uuid;

/// Context for evaluating binding match rules against incoming messages.
#[derive(Debug, Default)]
pub struct BindingContext {
    pub channel: String,
    pub account_id: Option<String>,
    pub peer_id: String,
    pub guild_id: Option<String>,
    pub roles: Vec<String>,
}

/// Routes incoming messages to the correct agent.
///
/// Routing priority: bindings (most specific first) > direct routes > user defaults > channel defaults > system default.
pub struct AgentRouter {
    user_defaults: DashMap<String, Uuid>,
    direct_routes: DashMap<(String, String), Uuid>,
    default_agent: Option<Uuid>,
    channel_defaults: DashMap<String, Uuid>,
    bindings: Mutex<Vec<(AgentBinding, String)>>,
    broadcast: Mutex<BroadcastConfig>,
    agent_name_cache: DashMap<String, Uuid>,
}

impl AgentRouter {
    pub fn new() -> Self {
        Self {
            user_defaults: DashMap::new(),
            direct_routes: DashMap::new(),
            default_agent: None,
            channel_defaults: DashMap::new(),
            bindings: Mutex::new(Vec::new()),
            broadcast: Mutex::new(BroadcastConfig::default()),
            agent_name_cache: DashMap::new(),
        }
    }

    pub fn set_default(&mut self, agent_id: Uuid) {
        self.default_agent = Some(agent_id);
    }

    pub fn set_channel_default(&self, channel_key: String, agent_id: Uuid) {
        self.channel_defaults.insert(channel_key, agent_id);
    }

    pub fn set_user_default(&self, user_key: String, agent_id: Uuid) {
        self.user_defaults.insert(user_key, agent_id);
    }

    pub fn set_direct_route(
        &self,
        channel_key: String,
        platform_user_id: String,
        agent_id: Uuid,
    ) {
        self.direct_routes
            .insert((channel_key, platform_user_id), agent_id);
    }

    pub fn load_bindings(&self, bindings: &[AgentBinding]) {
        let mut sorted: Vec<(AgentBinding, String)> = bindings
            .iter()
            .map(|b| (b.clone(), b.agent.clone()))
            .collect();
        sorted.sort_by(|a, b| {
            b.0.match_rule
                .specificity()
                .cmp(&a.0.match_rule.specificity())
        });
        *self.bindings.lock().unwrap_or_else(|e| e.into_inner()) = sorted;
    }

    pub fn load_broadcast(&self, broadcast: BroadcastConfig) {
        *self.broadcast.lock().unwrap_or_else(|e| e.into_inner()) = broadcast;
    }

    pub fn register_agent(&self, name: String, id: Uuid) {
        self.agent_name_cache.insert(name, id);
    }

    pub fn resolve(
        &self,
        channel_type: &ChannelType,
        platform_user_id: &str,
        user_key: Option<&str>,
    ) -> Option<Uuid> {
        let channel_key = channel_type.as_str().to_string();

        let ctx = BindingContext {
            channel: channel_key.clone(),
            account_id: None,
            peer_id: platform_user_id.to_string(),
            guild_id: None,
            roles: Vec::new(),
        };
        if let Some(agent_id) = self.resolve_binding(&ctx) {
            return Some(agent_id);
        }

        if let Some(agent) = self
            .direct_routes
            .get(&(channel_key.clone(), platform_user_id.to_string()))
        {
            return Some(*agent);
        }

        if let Some(key) = user_key {
            if let Some(agent) = self.user_defaults.get(key) {
                return Some(*agent);
            }
        }
        if let Some(agent) = self.user_defaults.get(platform_user_id) {
            return Some(*agent);
        }

        if let Some(agent) = self.channel_defaults.get(&channel_key) {
            return Some(*agent);
        }

        self.default_agent
    }

    pub fn resolve_with_context(
        &self,
        channel_type: &ChannelType,
        platform_user_id: &str,
        user_key: Option<&str>,
        ctx: &BindingContext,
    ) -> Option<Uuid> {
        if let Some(agent_id) = self.resolve_binding(ctx) {
            return Some(agent_id);
        }
        let channel_key = channel_type.as_str().to_string();
        if let Some(agent) = self
            .direct_routes
            .get(&(channel_key.clone(), platform_user_id.to_string()))
        {
            return Some(*agent);
        }
        if let Some(key) = user_key {
            if let Some(agent) = self.user_defaults.get(key) {
                return Some(*agent);
            }
        }
        if let Some(agent) = self.user_defaults.get(platform_user_id) {
            return Some(*agent);
        }
        if let Some(agent) = self.channel_defaults.get(&channel_key) {
            return Some(*agent);
        }
        self.default_agent
    }

    pub fn resolve_broadcast(&self, peer_id: &str) -> Vec<(String, Option<Uuid>)> {
        let bc = self.broadcast.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(agent_names) = bc.routes.get(peer_id) {
            agent_names
                .iter()
                .map(|name| {
                    let id = self.agent_name_cache.get(name).map(|r| *r);
                    (name.clone(), id)
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn broadcast_strategy(&self) -> BroadcastStrategy {
        self.broadcast
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .strategy
    }

    pub fn has_broadcast(&self, peer_id: &str) -> bool {
        self.broadcast
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .routes
            .contains_key(peer_id)
    }

    pub fn bindings(&self) -> Vec<AgentBinding> {
        self.bindings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(b, _)| b.clone())
            .collect()
    }

    pub fn add_binding(&self, binding: AgentBinding) {
        let name = binding.agent.clone();
        let mut bindings = self.bindings.lock().unwrap_or_else(|e| e.into_inner());
        bindings.push((binding, name));
        bindings.sort_by(|a, b| {
            b.0.match_rule
                .specificity()
                .cmp(&a.0.match_rule.specificity())
        });
    }

    pub fn remove_binding(&self, index: usize) -> Option<AgentBinding> {
        let mut bindings = self.bindings.lock().unwrap_or_else(|e| e.into_inner());
        if index < bindings.len() {
            Some(bindings.remove(index).0)
        } else {
            None
        }
    }

    fn resolve_binding(&self, ctx: &BindingContext) -> Option<Uuid> {
        let bindings = self.bindings.lock().unwrap_or_else(|e| e.into_inner());
        for (binding, _) in bindings.iter() {
            if self.binding_matches(binding, ctx) {
                if let Some(id) = self.agent_name_cache.get(&binding.agent) {
                    return Some(*id);
                }
                warn!(agent = %binding.agent, "Binding matched but agent not found in cache");
            }
        }
        None
    }

    fn binding_matches(&self, binding: &AgentBinding, ctx: &BindingContext) -> bool {
        let rule = &binding.match_rule;

        if let Some(ref ch) = rule.channel {
            if ch != &ctx.channel {
                return false;
            }
        }
        if let Some(ref acc) = rule.account_id {
            if ctx.account_id.as_ref() != Some(acc) {
                return false;
            }
        }
        if let Some(ref pid) = rule.peer_id {
            if pid != &ctx.peer_id {
                return false;
            }
        }
        if let Some(ref gid) = rule.guild_id {
            if ctx.guild_id.as_ref() != Some(gid) {
                return false;
            }
        }
        if !rule.roles.is_empty() {
            let has_role = rule.roles.iter().any(|r| ctx.roles.contains(r));
            if !has_role {
                return false;
            }
        }
        true
    }
}

impl Default for AgentRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BindingMatchRule;

    #[test]
    fn test_routing_priority() {
        let mut router = AgentRouter::new();
        let default_agent = Uuid::new_v4();
        let user_agent = Uuid::new_v4();
        let direct_agent = Uuid::new_v4();

        router.set_default(default_agent);
        router.set_user_default("alice".to_string(), user_agent);
        router.set_direct_route("telegram".to_string(), "tg_123".to_string(), direct_agent);

        let resolved = router.resolve(&ChannelType::Telegram, "tg_123", Some("alice"));
        assert_eq!(resolved, Some(direct_agent));

        let resolved = router.resolve(&ChannelType::WhatsApp, "wa_456", Some("alice"));
        assert_eq!(resolved, Some(user_agent));

        let resolved = router.resolve(&ChannelType::Discord, "dc_789", None);
        assert_eq!(resolved, Some(default_agent));
    }

    #[test]
    fn test_no_route() {
        let router = AgentRouter::new();
        let resolved = router.resolve(&ChannelType::CLI, "local", None);
        assert_eq!(resolved, None);
    }

    #[test]
    fn test_binding_channel_match() {
        let router = AgentRouter::new();
        let agent_id = Uuid::new_v4();
        router.register_agent("coder".to_string(), agent_id);
        router.load_bindings(&[AgentBinding {
            agent: "coder".to_string(),
            match_rule: BindingMatchRule {
                channel: Some("telegram".to_string()),
                ..Default::default()
            },
        }]);

        let resolved = router.resolve(&ChannelType::Telegram, "user1", None);
        assert_eq!(resolved, Some(agent_id));

        let resolved = router.resolve(&ChannelType::Discord, "user1", None);
        assert_eq!(resolved, None);
    }

    #[test]
    fn test_binding_specificity_ordering() {
        let router = AgentRouter::new();
        let general_id = Uuid::new_v4();
        let specific_id = Uuid::new_v4();
        router.register_agent("general".to_string(), general_id);
        router.register_agent("specific".to_string(), specific_id);

        router.load_bindings(&[
            AgentBinding {
                agent: "general".to_string(),
                match_rule: BindingMatchRule {
                    channel: Some("discord".to_string()),
                    ..Default::default()
                },
            },
            AgentBinding {
                agent: "specific".to_string(),
                match_rule: BindingMatchRule {
                    channel: Some("discord".to_string()),
                    peer_id: Some("user1".to_string()),
                    guild_id: Some("guild_1".to_string()),
                    ..Default::default()
                },
            },
        ]);

        let ctx = BindingContext {
            channel: "discord".to_string(),
            peer_id: "user1".to_string(),
            guild_id: Some("guild_1".to_string()),
            ..Default::default()
        };
        let resolved = router.resolve_with_context(&ChannelType::Discord, "user1", None, &ctx);
        assert_eq!(resolved, Some(specific_id));
    }

    #[test]
    fn test_broadcast_routing() {
        let router = AgentRouter::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        router.register_agent("agent-a".to_string(), id1);
        router.register_agent("agent-b".to_string(), id2);

        let mut routes = std::collections::HashMap::new();
        routes.insert(
            "vip_user".to_string(),
            vec!["agent-a".to_string(), "agent-b".to_string()],
        );
        router.load_broadcast(BroadcastConfig {
            strategy: BroadcastStrategy::Parallel,
            routes,
        });

        assert!(router.has_broadcast("vip_user"));
        assert!(!router.has_broadcast("normal_user"));

        let targets = router.resolve_broadcast("vip_user");
        assert_eq!(targets.len(), 2);
    }

    #[test]
    fn test_channel_default_routing() {
        let mut router = AgentRouter::new();
        let system_default = Uuid::new_v4();
        let telegram_default = Uuid::new_v4();

        router.set_default(system_default);
        router.set_channel_default("telegram".to_string(), telegram_default);

        let resolved = router.resolve(&ChannelType::Telegram, "user1", None);
        assert_eq!(resolved, Some(telegram_default));

        let resolved = router.resolve(&ChannelType::WhatsApp, "user1", None);
        assert_eq!(resolved, Some(system_default));
    }

    #[test]
    fn test_add_remove_binding() {
        let router = AgentRouter::new();
        let id = Uuid::new_v4();
        router.register_agent("test".to_string(), id);

        assert!(router.bindings().is_empty());

        router.add_binding(AgentBinding {
            agent: "test".to_string(),
            match_rule: BindingMatchRule {
                channel: Some("slack".to_string()),
                ..Default::default()
            },
        });
        assert_eq!(router.bindings().len(), 1);

        let removed = router.remove_binding(0);
        assert!(removed.is_some());
        assert!(router.bindings().is_empty());
    }
}
