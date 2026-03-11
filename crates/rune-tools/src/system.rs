use chrono::Utc;

pub fn system_time() -> serde_json::Value {
    let now = Utc::now();
    serde_json::json!({
        "iso8601": now.to_rfc3339(),
        "unix_epoch": now.timestamp(),
        "unix_epoch_ms": now.timestamp_millis(),
        "date": now.format("%Y-%m-%d").to_string(),
        "time": now.format("%H:%M:%S").to_string(),
        "timezone": "UTC",
    })
}

pub async fn location_get() -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let resp = client
        .get("http://ip-api.com/json/?fields=status,country,regionName,city,lat,lon,timezone,query")
        .header("User-Agent", "RuneAgent/0.1")
        .send()
        .await
        .map_err(|e| format!("Location request failed: {e}"))?;

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse location response: {e}"))?;

    if data["status"].as_str() == Some("fail") {
        return Err("IP geolocation lookup failed".into());
    }

    Ok(serde_json::json!({
        "ip": data["query"],
        "city": data["city"],
        "region": data["regionName"],
        "country": data["country"],
        "latitude": data["lat"],
        "longitude": data["lon"],
        "timezone": data["timezone"],
    }))
}
