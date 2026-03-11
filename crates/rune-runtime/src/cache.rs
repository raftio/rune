use std::sync::Arc;
use uuid::Uuid;

use rune_storage::{RuneStore, RuntimeStore};
use crate::error::RuntimeError;

/// Cache wrapper over RuneStore for session routing, replica load, and rate limiting.
#[derive(Clone)]
pub struct SqliteCache {
    store: Arc<RuneStore>,
}

impl SqliteCache {
    pub fn new(store: Arc<RuneStore>) -> Self {
        Self { store }
    }

    pub async fn get_session_route(&self, session_id: Uuid) -> Result<Option<Uuid>, RuntimeError> {
        Ok(self.store.get_session_route(session_id).await?)
    }

    pub async fn set_session_route(&self, session_id: Uuid, replica_id: Uuid, ttl_secs: Option<i64>) -> Result<(), RuntimeError> {
        Ok(self.store.set_session_route(session_id, replica_id, ttl_secs).await?)
    }

    pub async fn rate_limit_check(&self, key: &str, window_seconds: i64, max_count: i64) -> Result<bool, RuntimeError> {
        self.store.rate_limit_check(key, window_seconds, max_count).await.map_err(RuntimeError::Storage)
    }

    pub async fn cleanup_expired(&self) -> Result<u64, RuntimeError> {
        self.store.cleanup_expired_cache().await.map_err(RuntimeError::Storage)
    }
}
