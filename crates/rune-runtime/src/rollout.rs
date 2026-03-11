use sqlx::SqlitePool;
use uuid::Uuid;

use crate::models::Deployment;

/// Traffic weight for a deployment alias (0.0-1.0).
pub fn rollout_weight(rollout_alias: &str, canary_weight: f64) -> f64 {
    match rollout_alias {
        "canary" => canary_weight,
        "stable" | "default" => 1.0,
        _ => 1.0,
    }
}

pub fn is_canary_deployment(rollout_alias: &str) -> bool {
    rollout_alias == "canary"
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- rollout_weight ---

    #[test]
    fn canary_returns_configured_weight() {
        assert_eq!(rollout_weight("canary", 0.1), 0.1);
        assert_eq!(rollout_weight("canary", 0.25), 0.25);
        assert_eq!(rollout_weight("canary", 0.0), 0.0);
    }

    #[test]
    fn stable_always_returns_one() {
        assert_eq!(rollout_weight("stable", 0.1), 1.0);
        assert_eq!(rollout_weight("stable", 0.99), 1.0);
    }

    #[test]
    fn default_alias_returns_one() {
        assert_eq!(rollout_weight("default", 0.5), 1.0);
    }

    #[test]
    fn unknown_alias_returns_one() {
        assert_eq!(rollout_weight("blue-green", 0.5), 1.0);
        assert_eq!(rollout_weight("", 0.5), 1.0);
        assert_eq!(rollout_weight("v2", 0.3), 1.0);
    }

    // --- is_canary_deployment ---

    #[test]
    fn is_canary_true_for_canary() {
        assert!(is_canary_deployment("canary"));
    }

    #[test]
    fn is_canary_false_for_stable() {
        assert!(!is_canary_deployment("stable"));
    }

    #[test]
    fn is_canary_false_for_default() {
        assert!(!is_canary_deployment("default"));
    }

    #[test]
    fn is_canary_false_for_other() {
        assert!(!is_canary_deployment("v2"));
        assert!(!is_canary_deployment(""));
    }
}

pub struct RolloutManager {
    #[allow(dead_code)]
    db: SqlitePool,
    canary_weight: f64,
}

impl RolloutManager {
    pub fn new(db: SqlitePool, canary_weight: f64) -> Self {
        Self { db, canary_weight }
    }

    pub fn weight_for_deployment(&self, deployment: &Deployment) -> f64 {
        rollout_weight(&deployment.rollout_alias, self.canary_weight)
    }

    pub async fn rolling_replace_count(
        &self,
        _deployment_id: Uuid,
        desired: i64,
        healthy: i64,
    ) -> Result<(i64, &'static str), sqlx::Error> {
        if healthy > desired {
            Ok((healthy - desired, "excess replicas"))
        } else if healthy < desired {
            Ok((desired - healthy, "scaling up"))
        } else {
            Ok((0, "at desired"))
        }
    }
}
