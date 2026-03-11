use crate::ToolContext;
use uuid::Uuid;

pub async fn create(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let description = input["description"].as_str().ok_or("Missing 'description' parameter")?;
    let schedule_str = input["schedule"].as_str().ok_or("Missing 'schedule' parameter")?;
    let cron_expr = parse_schedule_to_cron(schedule_str)?;
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO schedules (id, description, schedule_input, cron_expr, enabled, created_at) VALUES (?, ?, ?, ?, 1, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
    )
    .bind(&id)
    .bind(description)
    .bind(schedule_str)
    .bind(&cron_expr)
    .execute(&ctx.db)
    .await
    .map_err(|e| format!("Database error: {e}"))?;

    Ok(serde_json::json!({
        "schedule_id": id,
        "description": description,
        "cron": cron_expr,
        "original": schedule_str,
    }))
}

pub async fn list(ctx: &ToolContext) -> Result<serde_json::Value, String> {
    let rows: Vec<(String, Option<String>, Option<String>, Option<String>, bool)> =
        sqlx::query_as(
            "SELECT id, description, schedule_input, cron_expr, enabled FROM schedules ORDER BY created_at DESC",
        )
        .fetch_all(&ctx.db)
        .await
        .map_err(|e| format!("Database error: {e}"))?;

    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|(id, desc, input, cron, enabled)| {
            serde_json::json!({
                "id": id,
                "description": desc,
                "schedule_input": input,
                "cron": cron,
                "enabled": enabled,
            })
        })
        .collect();

    Ok(serde_json::json!({ "count": items.len(), "schedules": items }))
}

pub async fn delete(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let id = input["id"].as_str().ok_or("Missing 'id' parameter")?;

    let rows = sqlx::query("DELETE FROM schedules WHERE id = ?")
        .bind(id)
        .execute(&ctx.db)
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .rows_affected();

    if rows == 0 {
        return Err(format!("Schedule '{id}' not found."));
    }
    Ok(serde_json::json!({ "message": format!("Schedule '{id}' deleted.") }))
}

pub async fn cron_create(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let schedule = input["schedule"].as_str().ok_or("Missing 'schedule' parameter")?;
    let action = input["action"].as_str().ok_or("Missing 'action' parameter")?;
    let cron_expr = parse_schedule_to_cron(schedule)?;
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO schedules (id, description, schedule_input, cron_expr, agent, enabled, created_at) VALUES (?, ?, ?, ?, ?, 1, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
    )
    .bind(&id)
    .bind(name)
    .bind(schedule)
    .bind(&cron_expr)
    .bind(action)
    .execute(&ctx.db)
    .await
    .map_err(|e| format!("Database error: {e}"))?;

    Ok(serde_json::json!({
        "job_id": id,
        "name": name,
        "cron": cron_expr,
    }))
}

pub async fn cron_list(ctx: &ToolContext) -> Result<serde_json::Value, String> {
    list(ctx).await
}

pub async fn cron_cancel(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let job_id = input["job_id"].as_str().ok_or("Missing 'job_id' parameter")?;

    let rows = sqlx::query("DELETE FROM schedules WHERE id = ?")
        .bind(job_id)
        .execute(&ctx.db)
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .rows_affected();

    if rows == 0 {
        return Err(format!("Cron job '{job_id}' not found."));
    }
    Ok(serde_json::json!({ "message": format!("Cron job '{job_id}' cancelled.") }))
}

fn parse_schedule_to_cron(input: &str) -> Result<String, String> {
    let input = input.trim().to_lowercase();

    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.len() == 5
        && parts
            .iter()
            .all(|p| p.chars().all(|c| c.is_ascii_digit() || "*/,-".contains(c)))
    {
        return Ok(input);
    }

    if let Some(rest) = input.strip_prefix("every ") {
        if rest == "minute" || rest == "1 minute" {
            return Ok("* * * * *".into());
        }
        if let Some(mins) = rest.strip_suffix(" minutes") {
            let n: u32 = mins.trim().parse().map_err(|_| format!("Invalid number in '{input}'"))?;
            if n == 0 || n > 59 { return Err(format!("Minutes must be 1-59, got {n}")); }
            return Ok(format!("*/{n} * * * *"));
        }
        if rest == "hour" || rest == "1 hour" {
            return Ok("0 * * * *".into());
        }
        if let Some(hrs) = rest.strip_suffix(" hours") {
            let n: u32 = hrs.trim().parse().map_err(|_| format!("Invalid number in '{input}'"))?;
            if n == 0 || n > 23 { return Err(format!("Hours must be 1-23, got {n}")); }
            return Ok(format!("0 */{n} * * *"));
        }
        if rest == "day" || rest == "1 day" { return Ok("0 0 * * *".into()); }
        if rest == "week" || rest == "1 week" { return Ok("0 0 * * 0".into()); }
    }

    if let Some(time_str) = input.strip_prefix("daily at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * *"));
    }
    if let Some(time_str) = input.strip_prefix("weekdays at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * 1-5"));
    }
    if let Some(time_str) = input.strip_prefix("weekends at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * 0,6"));
    }

    match input.as_str() {
        "hourly" => return Ok("0 * * * *".into()),
        "daily" => return Ok("0 0 * * *".into()),
        "weekly" => return Ok("0 0 * * 0".into()),
        "monthly" => return Ok("0 0 1 * *".into()),
        _ => {}
    }

    Err(format!(
        "Could not parse schedule '{input}'. Try: 'every 5 minutes', 'daily at 9am', or a cron expression."
    ))
}

fn parse_time_to_hour(s: &str) -> Result<u32, String> {
    let s = s.trim().to_lowercase();
    if let Some(h) = s.strip_suffix("am") {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        return match hour { 12 => Ok(0), 1..=11 => Ok(hour), _ => Err(format!("Invalid hour: {hour}")) };
    }
    if let Some(h) = s.strip_suffix("pm") {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        return match hour { 12 => Ok(12), 1..=11 => Ok(hour + 12), _ => Err(format!("Invalid hour: {hour}")) };
    }
    if let Some((h, _)) = s.split_once(':') {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        if hour > 23 { return Err(format!("Hour must be 0-23, got {hour}")); }
        return Ok(hour);
    }
    let hour: u32 = s.parse().map_err(|_| format!("Invalid time: {s}"))?;
    if hour > 23 { return Err(format!("Hour must be 0-23, got {hour}")); }
    Ok(hour)
}
