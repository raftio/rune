use bytes::Bytes;
use object_store::path::Path;
use object_store::{DynObjectStore, ObjectStore};
use std::sync::Arc;
use url::Url;
use uuid::Uuid;

use crate::error::StorageError;

const DEFAULT_CHECKPOINT_THRESHOLD_BYTES: usize = 1_048_576; // 1MB

pub struct ObjectStoreAdapter {
    store: Arc<DynObjectStore>,
    prefix: Path,
    checkpoint_threshold: usize,
}

impl ObjectStoreAdapter {
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("RUNE_OBJECT_STORE_URL").ok()?;
        let url = Url::parse(&url).ok()?;
        let (store, path) = object_store::parse_url(&url).ok()?;
        let threshold = std::env::var("RUNE_CHECKPOINT_STORAGE_THRESHOLD")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_CHECKPOINT_THRESHOLD_BYTES);

        Some(Self {
            store: Arc::new(store),
            prefix: path,
            checkpoint_threshold: threshold,
        })
    }

    pub fn from_platform_env(env: &rune_env::PlatformEnv) -> Option<Self> {
        let url_str = env.object_store_url.as_ref()?;
        let url = Url::parse(url_str).ok()?;
        let (store, path) = object_store::parse_url(&url).ok()?;

        Some(Self {
            store: Arc::new(store),
            prefix: path,
            checkpoint_threshold: env.checkpoint_storage_threshold,
        })
    }

    fn checkpoint_path(&self, session_id: Uuid) -> Path {
        let name = format!("{}.json", session_id);
        self.prefix.child("checkpoints").child(name.as_str())
    }

    pub async fn put_checkpoint(
        &self,
        session_id: Uuid,
        data: &[u8],
    ) -> Result<String, StorageError> {
        let path = self.checkpoint_path(session_id);
        self.store
            .put(&path, Bytes::copy_from_slice(data).into())
            .await
            .map_err(|e| StorageError::ObjectStore(format!("put: {e}")))?;
        Ok(path.to_string())
    }

    pub async fn get_checkpoint(&self, path_ref: &str) -> Result<Vec<u8>, StorageError> {
        let path = Path::from(path_ref);
        let result = self
            .store
            .get(&path)
            .await
            .map_err(|e| StorageError::ObjectStore(format!("get: {e}")))?;
        let bytes = result
            .bytes()
            .await
            .map_err(|e| StorageError::ObjectStore(format!("read: {e}")))?;
        Ok(bytes.to_vec())
    }

    pub fn should_store_in_object_store(&self, size: usize) -> bool {
        size >= self.checkpoint_threshold
    }
}
