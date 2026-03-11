use rune_storage::RuneStore;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::ControlPlaneError;

pub async fn check_deployment(
    store: &Arc<RuneStore>,
    agent_version_id: Uuid,
) -> Result<(), ControlPlaneError> {
    store.check_deployment_admission(agent_version_id).await.map_err(ControlPlaneError::Storage)
}
