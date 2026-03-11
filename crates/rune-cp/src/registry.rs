pub use rune_storage::ops::registry::{AgentVersionRow, RegisterVersionInput};

use rune_storage::RuneStore;
use std::sync::Arc;

use crate::error::ControlPlaneError;

pub async fn register(
    store: &Arc<RuneStore>,
    req: RegisterVersionInput,
) -> Result<AgentVersionRow, ControlPlaneError> {
    store.register_agent_version(&req).await.map_err(ControlPlaneError::Storage)
}

pub async fn list(store: &Arc<RuneStore>) -> Result<Vec<AgentVersionRow>, ControlPlaneError> {
    store.list_agent_versions().await.map_err(ControlPlaneError::Storage)
}
