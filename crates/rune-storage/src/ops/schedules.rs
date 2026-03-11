use sqlx::SqlitePool;

use crate::error::StorageError;

#[derive(sqlx::FromRow)]
pub struct ScheduleRow {
    pub id: String,
    pub cron_expr: Option<String>,
    pub agent: Option<String>,
}

pub async fn list_enabled(db: &SqlitePool) -> Result<Vec<ScheduleRow>, StorageError> {
    let rows = sqlx::query_as::<_, ScheduleRow>(
        "SELECT id, cron_expr, agent FROM schedules WHERE enabled = 1 AND cron_expr IS NOT NULL",
    )
    .fetch_all(db)
    .await?;
    Ok(rows)
}

pub async fn get_last_run(
    db: &SqlitePool,
    schedule_id: &str,
) -> Result<Option<String>, StorageError> {
    let last_run: Option<String> =
        sqlx::query_scalar("SELECT last_run_at FROM schedules WHERE id = ?")
            .bind(schedule_id)
            .fetch_optional(db)
            .await?;
    Ok(last_run)
}

pub async fn update_last_run(
    db: &SqlitePool,
    schedule_id: &str,
    now: &str,
) -> Result<(), StorageError> {
    sqlx::query("UPDATE schedules SET last_run_at = ? WHERE id = ?")
        .bind(now)
        .bind(schedule_id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn update_next_run(
    db: &SqlitePool,
    schedule_id: &str,
    next_run: &str,
) -> Result<(), StorageError> {
    sqlx::query("UPDATE schedules SET next_run_at = ? WHERE id = ?")
        .bind(next_run)
        .bind(schedule_id)
        .execute(db)
        .await?;
    Ok(())
}
