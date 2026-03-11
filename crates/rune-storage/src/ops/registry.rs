use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::StorageError;
use crate::models::AgentVersionImage;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct AgentVersionRow {
    pub id: String,
    pub agent_name: String,
    pub version: String,
    pub image_ref: String,
    pub image_digest: String,
    pub spec_sha256: String,
    pub signature_ref: Option<String>,
    pub runtime_class: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegisterVersionInput {
    pub agent_name: String,
    pub version: String,
    pub image_ref: String,
    pub image_digest: String,
    pub spec_sha256: String,
    pub signature_ref: Option<String>,
    pub runtime_class: String,
}

pub async fn register(
    db: &SqlitePool,
    input: &RegisterVersionInput,
) -> Result<AgentVersionRow, StorageError> {
    let id = Uuid::new_v4().to_string();
    let row = sqlx::query_as::<_, AgentVersionRow>(
        "INSERT INTO agent_versions
            (id, agent_name, version, image_ref, image_digest, spec_sha256,
             signature_ref, runtime_class, status)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'pending')
         ON CONFLICT (agent_name, version) DO UPDATE SET
            image_ref     = excluded.image_ref,
            image_digest  = excluded.image_digest,
            spec_sha256   = excluded.spec_sha256,
            signature_ref = excluded.signature_ref,
            runtime_class = excluded.runtime_class
         RETURNING *",
    )
    .bind(&id)
    .bind(&input.agent_name)
    .bind(&input.version)
    .bind(&input.image_ref)
    .bind(&input.image_digest)
    .bind(&input.spec_sha256)
    .bind(&input.signature_ref)
    .bind(&input.runtime_class)
    .fetch_one(db)
    .await?;
    Ok(row)
}

pub async fn list(db: &SqlitePool) -> Result<Vec<AgentVersionRow>, StorageError> {
    let rows = sqlx::query_as::<_, AgentVersionRow>(
        "SELECT * FROM agent_versions ORDER BY created_at DESC",
    )
    .fetch_all(db)
    .await?;
    Ok(rows)
}

pub async fn get_version_image(
    db: &SqlitePool,
    agent_version_id: Uuid,
) -> Result<Option<AgentVersionImage>, StorageError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        image_ref: String,
        image_digest: String,
        signature_ref: Option<String>,
    }
    let row = sqlx::query_as::<_, Row>(
        "SELECT image_ref, image_digest, signature_ref FROM agent_versions WHERE id = ?",
    )
    .bind(agent_version_id.to_string())
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| AgentVersionImage {
        image_ref: r.image_ref,
        image_digest: r.image_digest,
        signature_ref: r.signature_ref,
    }))
}

pub async fn check_admission(
    db: &SqlitePool,
    agent_version_id: Uuid,
) -> Result<(), StorageError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        image_ref: String,
        image_digest: String,
    }

    let row = sqlx::query_as::<_, Row>(
        "SELECT image_ref, image_digest FROM agent_versions WHERE id = ?",
    )
    .bind(agent_version_id.to_string())
    .fetch_optional(db)
    .await?;

    let Some(v) = row else {
        return Err(StorageError::NotFound(format!(
            "agent_version {} not found",
            agent_version_id
        )));
    };

    if v.image_digest.is_empty() {
        return Err(StorageError::NotFound(
            "image_digest is required for deployment".into(),
        ));
    }

    if v.image_ref.is_empty() {
        return Err(StorageError::NotFound(
            "image_ref is required for deployment".into(),
        ));
    }

    Ok(())
}

pub async fn version_exists(
    db: &SqlitePool,
    agent_version_id: Uuid,
) -> Result<bool, StorageError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_versions WHERE id = ?",
    )
    .bind(agent_version_id.to_string())
    .fetch_one(db)
    .await?;
    Ok(count > 0)
}
