use rune_storage::RuneStore;

use crate::error::NetworkError;
use crate::policy::NetworkPolicy;

pub struct NetworkRegistry {
    store: RuneStore,
}

impl NetworkRegistry {
    pub fn new(store: RuneStore) -> Self {
        Self { store }
    }

    pub async fn get_networks(&self, agent_name: &str) -> Result<Vec<String>, NetworkError> {
        self.store
            .get_agent_networks(agent_name)
            .await
            .map_err(|e| NetworkError::Storage(e))
    }

    pub async fn check_access(
        &self,
        caller: &str,
        caller_networks: &[String],
        callee: &str,
    ) -> Result<(), NetworkError> {
        let callee_networks = self.get_networks(callee).await?;

        let allowed = NetworkPolicy::check_access(caller_networks, &callee_networks);
        let resource = format!("{caller} -> {callee}");
        let deny_reason = if allowed { None } else { Some("network isolation policy") };

        if let Err(e) = self.store
            .audit_policy_decision("network", &resource, allowed, deny_reason, None)
            .await
        {
            tracing::warn!(error = %e, "failed to write network access audit record");
        }

        if allowed {
            tracing::debug!(
                caller,
                callee,
                ?caller_networks,
                ?callee_networks,
                "network access granted"
            );
            Ok(())
        } else {
            tracing::warn!(
                caller,
                callee,
                ?caller_networks,
                ?callee_networks,
                "network access denied"
            );
            Err(NetworkError::AccessDenied {
                caller: caller.to_string(),
                callee: callee.to_string(),
            })
        }
    }

    pub async fn direct_replica_endpoint(
        &self,
        agent_name: &str,
    ) -> Result<Option<String>, NetworkError> {
        self.store
            .get_agent_direct_replica_endpoint(agent_name)
            .await
            .map_err(|e| NetworkError::Storage(e))
    }
}
