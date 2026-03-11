use std::sync::Arc;
use uuid::Uuid;

use rune_storage::{RuneStore, RuntimeStore};
use crate::error::RuntimeError;

pub use rune_storage::{CheckpointBlob, SessionMessage as Message};

pub struct SessionManager {
    store: Arc<RuneStore>,
}

impl SessionManager {
    pub fn new(store: Arc<RuneStore>) -> Self {
        Self { store }
    }

    pub async fn create(
        &self,
        deployment_id: Uuid,
        tenant_id: Option<String>,
        routing_key: Option<String>,
    ) -> Result<Uuid, RuntimeError> {
        Ok(self.store
            .create_session(deployment_id, tenant_id.as_deref(), routing_key.as_deref())
            .await?)
    }

    pub async fn load_messages(&self, session_id: Uuid) -> Result<Vec<Message>, RuntimeError> {
        Ok(self.store.load_session_messages(session_id).await?)
    }

    pub async fn append_message(
        &self,
        session_id: Uuid,
        role: &str,
        content: serde_json::Value,
        step: i64,
    ) -> Result<(), RuntimeError> {
        Ok(self.store
            .append_session_message(session_id, role, &content, step)
            .await?)
    }

    pub async fn close(&self, session_id: Uuid) -> Result<(), RuntimeError> {
        Ok(self.store.close_session(session_id).await?)
    }

    pub async fn get_deployment_id(
        &self,
        session_id: Uuid,
    ) -> Result<Option<Uuid>, RuntimeError> {
        Ok(self.store.get_session_deployment_id(session_id).await?)
    }

    pub async fn checkpoint(
        &self,
        session_id: Uuid,
        messages: &[Message],
        step: i64,
    ) -> Result<(), RuntimeError> {
        Ok(self.store
            .checkpoint_session(session_id, messages, step)
            .await?)
    }

    pub async fn restore(
        &self,
        session_id: Uuid,
    ) -> Result<Option<(Vec<Message>, i64)>, RuntimeError> {
        Ok(self.store.restore_session(session_id).await?)
    }

    pub async fn apply_restore(
        &self,
        session_id: Uuid,
        messages: &[Message],
    ) -> Result<(), RuntimeError> {
        Ok(self.store
            .apply_session_restore(session_id, messages)
            .await?)
    }
}
