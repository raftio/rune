use crate::ToolContext;
use uuid::Uuid;

pub async fn add_entity(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let entity_type = input["entity_type"].as_str().ok_or("Missing 'entity_type' parameter")?;
    let properties = input.get("properties").cloned().unwrap_or(serde_json::json!({}));
    let props_str = serde_json::to_string(&properties).unwrap_or_default();
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO kg_entities (id, name, entity_type, properties, created_at) VALUES (?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
    )
    .bind(&id)
    .bind(name)
    .bind(entity_type)
    .bind(&props_str)
    .execute(&ctx.db)
    .await
    .map_err(|e| format!("Database error: {e}"))?;

    Ok(serde_json::json!({
        "id": id,
        "message": format!("Entity '{name}' ({entity_type}) added.")
    }))
}

pub async fn add_relation(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let source = input["source"].as_str().ok_or("Missing 'source' parameter")?;
    let relation = input["relation"].as_str().ok_or("Missing 'relation' parameter")?;
    let target = input["target"].as_str().ok_or("Missing 'target' parameter")?;
    let confidence = input["confidence"].as_f64().unwrap_or(1.0);
    let properties = input.get("properties").cloned().unwrap_or(serde_json::json!({}));
    let props_str = serde_json::to_string(&properties).unwrap_or_default();
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO kg_relations (id, source, relation, target, confidence, properties, created_at) VALUES (?, ?, ?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
    )
    .bind(&id)
    .bind(source)
    .bind(relation)
    .bind(target)
    .bind(confidence)
    .bind(&props_str)
    .execute(&ctx.db)
    .await
    .map_err(|e| format!("Database error: {e}"))?;

    Ok(serde_json::json!({
        "id": id,
        "message": format!("Relation '{source}' --[{relation}]--> '{target}' added.")
    }))
}

pub async fn query(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let source = input["source"].as_str();
    let relation = input["relation"].as_str();
    let target = input["target"].as_str();

    let mut sql = String::from(
        "SELECT r.id, r.source, r.relation, r.target, r.confidence,
                se.name as source_name, se.entity_type as source_type,
                te.name as target_name, te.entity_type as target_type
         FROM kg_relations r
         LEFT JOIN kg_entities se ON (r.source = se.id OR r.source = se.name)
         LEFT JOIN kg_entities te ON (r.target = te.id OR r.target = te.name)
         WHERE 1=1",
    );
    let mut binds: Vec<String> = Vec::new();

    if let Some(s) = source {
        sql.push_str(" AND (r.source = ? OR se.name = ?)");
        binds.push(s.to_string());
        binds.push(s.to_string());
    }
    if let Some(r) = relation {
        sql.push_str(" AND r.relation = ?");
        binds.push(r.to_string());
    }
    if let Some(t) = target {
        sql.push_str(" AND (r.target = ? OR te.name = ?)");
        binds.push(t.to_string());
        binds.push(t.to_string());
    }
    sql.push_str(" LIMIT 100");

    let mut q = sqlx::query_as::<_, (String, String, String, String, f64, Option<String>, Option<String>, Option<String>, Option<String>)>(&sql);
    for b in &binds {
        q = q.bind(b);
    }

    let rows = q
        .fetch_all(&ctx.db)
        .await
        .map_err(|e| format!("Database error: {e}"))?;

    let results: Vec<serde_json::Value> = rows
        .iter()
        .map(|(id, src, rel, tgt, conf, sn, st, tn, tt)| {
            serde_json::json!({
                "id": id,
                "source": { "ref": src, "name": sn, "type": st },
                "relation": rel,
                "target": { "ref": tgt, "name": tn, "type": tt },
                "confidence": conf,
            })
        })
        .collect();

    Ok(serde_json::json!({ "count": results.len(), "results": results }))
}
