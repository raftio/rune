use rune_spec::ModelsSpec;
use rune_storage::RuneStore;
use std::collections::HashSet;

use crate::error::RuntimeError;

#[derive(Debug)]
pub enum PolicyDecision {
    Allow,
    Deny { reason: String },
}

pub struct PolicyEngine {
    allowed_tools: HashSet<String>,
    models: ModelsSpec,
}

impl PolicyEngine {
    pub fn new(allowed_tools: impl IntoIterator<Item = String>, models: ModelsSpec) -> Self {
        Self {
            allowed_tools: allowed_tools.into_iter().collect(),
            models,
        }
    }

    pub fn check_tool(&self, tool_name: &str) -> PolicyDecision {
        let canonical = tool_name.strip_prefix("rune__").unwrap_or(tool_name);
        let alt = format!("rune@{}", canonical);
        if self.allowed_tools.is_empty() {
            return PolicyDecision::Allow;
        }
        if self.allowed_tools.contains(tool_name)
            || self.allowed_tools.contains(canonical)
            || self.allowed_tools.contains(&alt)
        {
            PolicyDecision::Allow
        } else {
            PolicyDecision::Deny {
                reason: format!("tool '{}' not in agent toolset", tool_name),
            }
        }
    }

    pub fn check_model(&self, model: &str) -> PolicyDecision {
        if self.models.providers.is_empty() {
            return PolicyDecision::Allow;
        }
        let actual_model = self.models
            .model_mapping
            .get(model)
            .map(|s| s.as_str())
            .unwrap_or(model);
        let provider = model_to_provider(actual_model);
        if self.models.providers.iter().any(|p| p.eq_ignore_ascii_case(&provider)) {
            PolicyDecision::Allow
        } else {
            PolicyDecision::Deny {
                reason: format!(
                    "model '{}' (provider {}) not in allowlist {:?}",
                    model, provider, self.models.providers
                ),
            }
        }
    }
}

fn model_to_provider(model: &str) -> &'static str {
    let m = model.to_lowercase();
    if m.contains("claude") || m.contains("anthropic") {
        "anthropic"
    } else if m.contains("gpt") || m.contains("o1") || m.contains("openai") {
        "openai"
    } else if m.contains("gemini") || m.contains("google") {
        "google"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune_spec::ModelsSpec;
    use std::collections::HashMap;

    fn make_engine(tools: &[&str], providers: &[&str]) -> PolicyEngine {
        let models = ModelsSpec {
            providers: providers.iter().map(|s| s.to_string()).collect(),
            model_mapping: HashMap::new(),
            fallback_policy: rune_spec::models::FallbackPolicy::NextProvider,
            token_budget: 100_000,
            safety_policy: rune_spec::models::SafetyPolicy::Standard,
        };
        PolicyEngine::new(tools.iter().map(|s| s.to_string()), models)
    }

    // --- check_tool ---

    #[test]
    fn check_tool_empty_allowlist_allows_all() {
        let engine = make_engine(&[], &[]);
        assert!(matches!(engine.check_tool("anything"), PolicyDecision::Allow));
        assert!(matches!(engine.check_tool("rune@shell"), PolicyDecision::Allow));
    }

    #[test]
    fn check_tool_exact_match_allows() {
        let engine = make_engine(&["my_tool"], &[]);
        assert!(matches!(engine.check_tool("my_tool"), PolicyDecision::Allow));
    }

    #[test]
    fn check_tool_not_in_list_denies() {
        let engine = make_engine(&["allowed_tool"], &[]);
        let decision = engine.check_tool("other_tool");
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
        if let PolicyDecision::Deny { reason } = decision {
            assert!(reason.contains("other_tool"));
            assert!(reason.contains("toolset"));
        }
    }

    #[test]
    fn check_tool_rune_double_underscore_strips_to_canonical() {
        // "rune__shell" → canonical = "shell" → alt = "rune@shell"
        let engine = make_engine(&["rune@shell"], &[]);
        assert!(matches!(engine.check_tool("rune__shell"), PolicyDecision::Allow));
    }

    #[test]
    fn check_tool_rune_at_prefix_allowed_directly() {
        let engine = make_engine(&["rune@file-read"], &[]);
        assert!(matches!(engine.check_tool("rune@file-read"), PolicyDecision::Allow));
    }

    #[test]
    fn check_tool_canonical_resolves_rune_at_alt() {
        // tool_name "file-read" → no prefix → canonical="file-read" → alt="rune@file-read"
        let engine = make_engine(&["rune@file-read"], &[]);
        assert!(matches!(engine.check_tool("file-read"), PolicyDecision::Allow));
    }

    #[test]
    fn check_tool_deny_message_contains_tool_name() {
        let engine = make_engine(&["other"], &[]);
        if let PolicyDecision::Deny { reason } = engine.check_tool("secret_tool") {
            assert!(reason.contains("secret_tool"));
        } else {
            panic!("expected Deny");
        }
    }

    #[test]
    fn check_tool_multiple_tools_in_allowlist() {
        let engine = make_engine(&["tool_a", "rune@shell", "tool_b"], &[]);
        assert!(matches!(engine.check_tool("tool_a"), PolicyDecision::Allow));
        assert!(matches!(engine.check_tool("rune@shell"), PolicyDecision::Allow));
        assert!(matches!(engine.check_tool("tool_b"), PolicyDecision::Allow));
        assert!(matches!(engine.check_tool("tool_c"), PolicyDecision::Deny { .. }));
    }

    // --- check_model ---

    #[test]
    fn check_model_empty_providers_allows_all() {
        let engine = make_engine(&[], &[]);
        assert!(matches!(engine.check_model("claude-sonnet-4-6"), PolicyDecision::Allow));
        assert!(matches!(engine.check_model("gpt-4o"), PolicyDecision::Allow));
        assert!(matches!(engine.check_model("unknown-model"), PolicyDecision::Allow));
    }

    #[test]
    fn check_model_claude_maps_to_anthropic() {
        let engine = make_engine(&[], &["anthropic"]);
        assert!(matches!(engine.check_model("claude-sonnet-4-6"), PolicyDecision::Allow));
        assert!(matches!(engine.check_model("claude-opus-4-6"), PolicyDecision::Allow));
    }

    #[test]
    fn check_model_gpt_maps_to_openai() {
        let engine = make_engine(&[], &["openai"]);
        assert!(matches!(engine.check_model("gpt-4o"), PolicyDecision::Allow));
        assert!(matches!(engine.check_model("gpt-4o-mini"), PolicyDecision::Allow));
        assert!(matches!(engine.check_model("o1"), PolicyDecision::Allow));
    }

    #[test]
    fn check_model_gemini_maps_to_google() {
        let engine = make_engine(&[], &["google"]);
        assert!(matches!(engine.check_model("gemini-pro"), PolicyDecision::Allow));
        assert!(matches!(engine.check_model("gemini-1.5-flash"), PolicyDecision::Allow));
    }

    #[test]
    fn check_model_provider_not_in_list_denies() {
        let engine = make_engine(&[], &["anthropic"]);
        let decision = engine.check_model("gpt-4o");
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
        if let PolicyDecision::Deny { reason } = decision {
            assert!(reason.contains("gpt-4o"));
            assert!(reason.contains("anthropic"));
        }
    }

    #[test]
    fn check_model_provider_match_is_case_insensitive() {
        let engine = make_engine(&[], &["Anthropic"]);
        assert!(matches!(engine.check_model("claude-sonnet-4-6"), PolicyDecision::Allow));
    }

    #[test]
    fn check_model_unknown_model_denied_when_providers_set() {
        let engine = make_engine(&[], &["anthropic", "openai"]);
        let decision = engine.check_model("some-local-model");
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }

    #[test]
    fn check_model_multiple_providers_both_allowed() {
        let engine = make_engine(&[], &["anthropic", "openai"]);
        assert!(matches!(engine.check_model("claude-sonnet-4-6"), PolicyDecision::Allow));
        assert!(matches!(engine.check_model("gpt-4o"), PolicyDecision::Allow));
    }

    #[test]
    fn check_model_alias_resolved_via_mapping() {
        let mut model_mapping = HashMap::new();
        model_mapping.insert("default".into(), "gpt-4o-mini".into());
        let engine = PolicyEngine::new(
            std::iter::empty::<String>(),
            ModelsSpec {
                providers: vec!["openai".into()],
                model_mapping,
                fallback_policy: rune_spec::models::FallbackPolicy::NextProvider,
                token_budget: 100_000,
                safety_policy: rune_spec::models::SafetyPolicy::Standard,
            },
        );
        assert!(matches!(engine.check_model("default"), PolicyDecision::Allow));
    }
}

pub async fn audit_policy_decision(
    store: &RuneStore,
    decision: &PolicyDecision,
    kind: &str,
    resource: &str,
    request_id: Option<&str>,
) -> Result<(), RuntimeError> {
    let (allowed, reason) = match decision {
        PolicyDecision::Allow => (true, None),
        PolicyDecision::Deny { reason } => (false, Some(reason.as_str())),
    };
    store
        .audit_policy_decision(kind, resource, allowed, reason, request_id)
        .await?;
    Ok(())
}
