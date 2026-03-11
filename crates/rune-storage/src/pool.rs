use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

use crate::error::StorageError;

#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub database_url: String,
    pub max_connections: u32,
    pub create_if_missing: bool,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            database_url: "sqlite:rune.db".into(),
            max_connections: 4,
            create_if_missing: true,
        }
    }
}

pub async fn create_pool(config: &PoolConfig) -> Result<SqlitePool, StorageError> {
    let opts = SqliteConnectOptions::from_str(&config.database_url)
        .map_err(|e| StorageError::Database(e))?
        .create_if_missing(config.create_if_missing)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(config.max_connections)
        .connect_with(opts)
        .await?;

    Ok(pool)
}
