use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelsSpec {
    #[serde(default)]
    pub providers: Vec<String>,
    #[serde(default)]
    pub model_mapping: HashMap<String, String>,
    #[serde(default)]
    pub fallback_policy: FallbackPolicy,
    #[serde(default = "default_token_budget")]
    pub token_budget: u32,
    #[serde(default)]
    pub safety_policy: SafetyPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FallbackPolicy {
    #[default]
    NextProvider,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SafetyPolicy {
    #[default]
    Standard,
    Strict,
    None,
}

fn default_token_budget() -> u32 { 100_000 }

impl ModelsSpec {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_uses_all_defaults() {
        let spec: ModelsSpec = serde_yaml::from_str("{}").unwrap();
        assert!(spec.providers.is_empty());
        assert!(spec.model_mapping.is_empty());
        assert!(matches!(spec.fallback_policy, FallbackPolicy::NextProvider));
        assert_eq!(spec.token_budget, 100_000);
        assert!(matches!(spec.safety_policy, SafetyPolicy::Standard));
    }

    #[test]
    fn parse_full() {
        let yaml = r#"
providers:
  - anthropic
  - openai
model_mapping:
  default: claude-sonnet-4-6
  fast: claude-haiku-4-5
fallback_policy: fail
token_budget: 50000
safety_policy: strict
"#;
        let spec: ModelsSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.providers, vec!["anthropic", "openai"]);
        assert_eq!(spec.model_mapping["default"], "claude-sonnet-4-6");
        assert_eq!(spec.model_mapping["fast"], "claude-haiku-4-5");
        assert!(matches!(spec.fallback_policy, FallbackPolicy::Fail));
        assert_eq!(spec.token_budget, 50_000);
        assert!(matches!(spec.safety_policy, SafetyPolicy::Strict));
    }

    #[test]
    fn fallback_policy_variants() {
        for (val, expected) in &[("next_provider", "NextProvider"), ("fail", "Fail")] {
            let yaml = format!("fallback_policy: {val}");
            let spec: ModelsSpec = serde_yaml::from_str(&yaml).unwrap();
            assert!(format!("{:?}", spec.fallback_policy).contains(expected));
        }
    }

    #[test]
    fn safety_policy_variants() {
        for (val, expected) in &[
            ("standard", "Standard"),
            ("strict", "Strict"),
            ("none", "None"),
        ] {
            let yaml = format!("safety_policy: {val}");
            let spec: ModelsSpec = serde_yaml::from_str(&yaml).unwrap();
            assert!(format!("{:?}", spec.safety_policy).contains(expected));
        }
    }

    #[test]
    fn providers_order_preserved() {
        let yaml = "providers:\n  - openai\n  - anthropic\n  - local\n";
        let spec: ModelsSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.providers, vec!["openai", "anthropic", "local"]);
    }

    #[test]
    fn multiple_model_mappings() {
        let yaml = "model_mapping:\n  default: gpt-4o\n  cheap: gpt-4o-mini\n  reasoning: o1\n";
        let spec: ModelsSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.model_mapping.len(), 3);
        assert_eq!(spec.model_mapping["reasoning"], "o1");
    }

}
