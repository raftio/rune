use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSpec {
    #[serde(default = "default_concurrency_limit")]
    pub concurrency_limit: u32,
    #[serde(default)]
    pub health_probe: HealthProbe,
    #[serde(default = "default_startup_timeout_ms")]
    pub startup_timeout_ms: u64,
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default = "default_true")]
    pub streaming_enabled: bool,
    #[serde(default)]
    pub checkpoint_policy: CheckpointPolicy,
    #[serde(default)]
    pub resource_profile: ResourceProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthProbe {
    pub path: String,
    pub interval_ms: u64,
}

impl Default for HealthProbe {
    fn default() -> Self {
        Self { path: "/health".into(), interval_ms: 5_000 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointPolicy {
    #[default]
    OnFinish,
    Never,
    EachStep,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResourceProfile {
    #[default]
    Small,
    Medium,
    Large,
}

fn default_concurrency_limit() -> u32 { 10 }
fn default_startup_timeout_ms() -> u64 { 10_000 }
fn default_request_timeout_ms() -> u64 { 30_000 }
fn default_true() -> bool { true }

impl RuntimeSpec {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_uses_all_defaults() {
        let spec: RuntimeSpec = serde_yaml::from_str("{}").unwrap();
        assert_eq!(spec.concurrency_limit, 10);
        assert_eq!(spec.health_probe.path, "/health");
        assert_eq!(spec.health_probe.interval_ms, 5_000);
        assert_eq!(spec.startup_timeout_ms, 10_000);
        assert_eq!(spec.request_timeout_ms, 30_000);
        assert!(spec.streaming_enabled);
        assert!(matches!(spec.checkpoint_policy, CheckpointPolicy::OnFinish));
        assert!(matches!(spec.resource_profile, ResourceProfile::Small));
    }

    #[test]
    fn parse_full() {
        let yaml = r#"
concurrency_limit: 5
health_probe:
  path: /readyz
  interval_ms: 10000
startup_timeout_ms: 20000
request_timeout_ms: 60000
streaming_enabled: false
checkpoint_policy: each_step
resource_profile: large
"#;
        let spec: RuntimeSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.concurrency_limit, 5);
        assert_eq!(spec.health_probe.path, "/readyz");
        assert_eq!(spec.health_probe.interval_ms, 10_000);
        assert_eq!(spec.startup_timeout_ms, 20_000);
        assert_eq!(spec.request_timeout_ms, 60_000);
        assert!(!spec.streaming_enabled);
        assert!(matches!(spec.checkpoint_policy, CheckpointPolicy::EachStep));
        assert!(matches!(spec.resource_profile, ResourceProfile::Large));
    }

    #[test]
    fn checkpoint_policy_variants() {
        for (val, expected) in &[
            ("on_finish", "OnFinish"),
            ("never", "Never"),
            ("each_step", "EachStep"),
        ] {
            let yaml = format!("checkpoint_policy: {val}");
            let spec: RuntimeSpec = serde_yaml::from_str(&yaml).unwrap();
            assert!(format!("{:?}", spec.checkpoint_policy).contains(expected));
        }
    }

    #[test]
    fn resource_profile_variants() {
        for (val, expected) in &[("small", "Small"), ("medium", "Medium"), ("large", "Large")] {
            let yaml = format!("resource_profile: {val}");
            let spec: RuntimeSpec = serde_yaml::from_str(&yaml).unwrap();
            assert!(format!("{:?}", spec.resource_profile).contains(expected));
        }
    }

    #[test]
    fn health_probe_partial_override() {
        let yaml = "health_probe:\n  path: /livez\n  interval_ms: 2000\n";
        let spec: RuntimeSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.health_probe.path, "/livez");
        assert_eq!(spec.health_probe.interval_ms, 2_000);
    }

    #[test]
    fn streaming_can_be_disabled() {
        let spec: RuntimeSpec = serde_yaml::from_str("streaming_enabled: false").unwrap();
        assert!(!spec.streaming_enabled);
    }

}
