use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::StorageError;

pub async fn insert_invocation(
    db: &SqlitePool,
    invocation_id: &str,
    request_id: Uuid,
    tool_name: &str,
    tool_runtime: &str,
    input_payload: &str,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO tool_invocations
            (id, request_id, tool_name, tool_runtime, input_payload, status)
         VALUES (?, ?, ?, ?, ?, 'running')",
    )
    .bind(invocation_id)
    .bind(request_id.to_string())
    .bind(tool_name)
    .bind(tool_runtime)
    .bind(input_payload)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn update_invocation_completed(
    db: &SqlitePool,
    invocation_id: &str,
    output_payload: &str,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE tool_invocations
         SET status = 'completed',
             output_payload = ?,
             completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?",
    )
    .bind(output_payload)
    .bind(invocation_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn update_invocation_failed(
    db: &SqlitePool,
    invocation_id: &str,
    error_message: &str,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE tool_invocations
         SET status = 'failed',
             output_payload = ?,
             completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?",
    )
    .bind(error_message)
    .bind(invocation_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn insert_agent_call(
    db: &SqlitePool,
    call_id: &str,
    caller_task_id: &str,
    caller_agent: &str,
    callee_agent: &str,
    callee_endpoint: &str,
    depth: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO agent_calls (id, caller_task_id, caller_agent, callee_agent, callee_endpoint, depth, status)
         VALUES (?, ?, ?, ?, ?, ?, 'running')",
    )
    .bind(call_id)
    .bind(caller_task_id)
    .bind(caller_agent)
    .bind(callee_agent)
    .bind(callee_endpoint)
    .bind(depth)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn update_agent_call_callee_task(
    db: &SqlitePool,
    call_id: &str,
    callee_task_id: &str,
) -> Result<(), StorageError> {
    sqlx::query("UPDATE agent_calls SET callee_task_id = ? WHERE id = ?")
        .bind(callee_task_id)
        .bind(call_id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn update_agent_call_completed(
    db: &SqlitePool,
    call_id: &str,
    latency_ms: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE agent_calls SET status = 'completed', completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), latency_ms = ?
         WHERE id = ?",
    )
    .bind(latency_ms)
    .bind(call_id)
    .execute(db)
    .await?;
    Ok(())
}
