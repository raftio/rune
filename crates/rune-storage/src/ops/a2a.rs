use sqlx::SqlitePool;
use rune_store_runtime::{A2aTaskRow, HistoryMessage};

use crate::error::StorageError;

pub async fn insert_task(
    db: &SqlitePool,
    task_id: &str,
    context_id: &str,
    request_id: &str,
    session_id: &str,
    agent_name: &str,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO a2a_tasks (task_id, context_id, request_id, session_id, agent_name, state)
         VALUES (?, ?, ?, ?, ?, 'submitted')",
    )
    .bind(task_id)
    .bind(context_id)
    .bind(request_id)
    .bind(session_id)
    .bind(agent_name)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn update_state(
    db: &SqlitePool,
    task_id: &str,
    state: &str,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE a2a_tasks SET state = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE task_id = ?",
    )
    .bind(state)
    .bind(task_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn get_task(
    db: &SqlitePool,
    task_id: &str,
) -> Result<Option<A2aTaskRow>, StorageError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        task_id: String,
        context_id: String,
        request_id: String,
        session_id: String,
        agent_name: String,
        state: String,
        created_at: String,
        updated_at: String,
    }

    let row: Option<Row> = sqlx::query_as(
        "SELECT task_id, context_id, request_id, session_id, agent_name, state, created_at, updated_at
         FROM a2a_tasks WHERE task_id = ?",
    )
    .bind(task_id)
    .fetch_optional(db)
    .await?;

    Ok(row.map(|r| A2aTaskRow {
        task_id: r.task_id,
        context_id: r.context_id,
        request_id: r.request_id,
        session_id: r.session_id,
        agent_name: r.agent_name,
        state: r.state,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }))
}

pub async fn cancel_task(db: &SqlitePool, task_id: &str) -> Result<u64, StorageError> {
    let rows_affected = sqlx::query(
        "UPDATE a2a_tasks SET state = 'canceled', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE task_id = ? AND state NOT IN ('completed', 'failed', 'canceled')",
    )
    .bind(task_id)
    .execute(db)
    .await?
    .rows_affected();

    if rows_affected > 0 {
        sqlx::query(
            "UPDATE agent_requests SET status = 'canceled',
             completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = (SELECT request_id FROM a2a_tasks WHERE task_id = ?)",
        )
        .bind(task_id)
        .execute(db)
        .await?;
    }

    Ok(rows_affected)
}

pub async fn load_history(
    db: &SqlitePool,
    session_id: &str,
    limit: i64,
) -> Result<Vec<HistoryMessage>, StorageError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        role: String,
        content: String,
        created_at: String,
    }

    let rows: Vec<Row> = sqlx::query_as(
        "SELECT role, content, created_at FROM session_messages
         WHERE session_id = ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(session_id)
    .bind(limit)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| HistoryMessage {
            role: r.role,
            content: r.content,
            created_at: r.created_at,
        })
        .collect())
}
