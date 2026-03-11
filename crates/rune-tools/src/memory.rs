use crate::ToolContext;

pub async fn store(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    let value = input
        .get("value")
        .ok_or("Missing 'value' parameter")?;
    let value_str = if let Some(s) = value.as_str() {
        s.to_string()
    } else {
        serde_json::to_string(value).map_err(|e| format!("Failed to serialize value: {e}"))?
    };

    sqlx::query(
        "INSERT INTO rune_kv (key, value, updated_at) VALUES (?, ?, strftime('%Y-%m-%dT%H:%M:%fZ','now'))
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(&value_str)
    .execute(&ctx.db)
    .await
    .map_err(|e| format!("Database error: {e}"))?;

    Ok(serde_json::json!({ "message": format!("Stored value under key '{key}'.") }))
}

pub async fn list(
    _input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT key FROM rune_kv ORDER BY key")
            .fetch_all(&ctx.db)
            .await
            .map_err(|e| format!("Database error: {e}"))?;

    let keys: Vec<String> = rows.into_iter().map(|(k,)| k).collect();
    Ok(serde_json::json!({ "keys": keys, "count": keys.len() }))
}

pub async fn recall(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;

    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM rune_kv WHERE key = ?")
            .bind(key)
            .fetch_optional(&ctx.db)
            .await
            .map_err(|e| format!("Database error: {e}"))?;

    match row {
        Some((value,)) => {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&value) {
                Ok(serde_json::json!({ "key": key, "value": parsed }))
            } else {
                Ok(serde_json::json!({ "key": key, "value": value }))
            }
        }
        None => Ok(serde_json::json!({ "key": key, "value": null, "message": format!("No value found for key '{key}'.") })),
    }
}
