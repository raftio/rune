use std::net::{IpAddr, ToSocketAddrs};

pub async fn web_search(input: &serde_json::Value) -> Result<serde_json::Value, String> {
    let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
    let max_results = input["max_results"].as_u64().unwrap_or(5).min(20) as usize;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let resp = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .header("User-Agent", "Mozilla/5.0 (compatible; RuneAgent/0.1)")
        .send()
        .await
        .map_err(|e| format!("Search request failed: {e}"))?;

    let body = resp.text().await.map_err(|e| format!("Failed to read response: {e}"))?;
    let results = parse_ddg_results(&body, max_results);

    if results.is_empty() {
        return Ok(serde_json::json!({
            "query": query,
            "results": [],
            "message": format!("No results found for '{query}'.")
        }));
    }

    let items: Vec<serde_json::Value> = results
        .iter()
        .enumerate()
        .map(|(i, (title, url, snippet))| {
            serde_json::json!({
                "rank": i + 1,
                "title": title,
                "url": url,
                "snippet": snippet,
            })
        })
        .collect();

    Ok(serde_json::json!({ "query": query, "results": items }))
}

pub async fn web_fetch(input: &serde_json::Value) -> Result<serde_json::Value, String> {
    let url = input["url"].as_str().ok_or("Missing 'url' parameter")?;
    let method = input["method"].as_str().unwrap_or("GET").to_uppercase();

    check_ssrf(url)?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let mut req = match method.as_str() {
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "PATCH" => client.patch(url),
        "DELETE" => client.delete(url),
        _ => client.get(url),
    };
    req = req.header("User-Agent", "Mozilla/5.0 (compatible; RuneAgent/0.1)");

    if let Some(headers) = input["headers"].as_object() {
        for (k, v) in headers {
            if let Some(val) = v.as_str() {
                req = req.header(k.as_str(), val);
            }
        }
    }
    if let Some(body) = input["body"].as_str() {
        if body.trim_start().starts_with('{') || body.trim_start().starts_with('[') {
            req = req.header("Content-Type", "application/json");
        }
        req = req.body(body.to_string());
    }

    let resp = req.send().await.map_err(|e| format!("HTTP request failed: {e}"))?;
    let status = resp.status().as_u16();

    if let Some(len) = resp.content_length() {
        if len > 10 * 1024 * 1024 {
            return Err(format!("Response too large: {len} bytes (max 10MB)"));
        }
    }

    let body = resp.text().await.map_err(|e| format!("Failed to read body: {e}"))?;

    const MAX_CHARS: usize = 50_000;
    let truncated = body.len() > MAX_CHARS;
    let content = if truncated { &body[..MAX_CHARS] } else { &body };

    Ok(serde_json::json!({
        "status": status,
        "body": content,
        "truncated": truncated,
        "total_bytes": body.len(),
    }))
}

fn check_ssrf(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;

    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("Blocked scheme: {other} (only http/https allowed)")),
    }

    let host = parsed.host_str().ok_or("URL missing host")?;

    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_ip(ip) {
            return Err(format!("SSRF blocked: private IP {ip}"));
        }
    }

    if let Ok(addrs) = (host, parsed.port().unwrap_or(80)).to_socket_addrs() {
        for addr in addrs {
            if is_private_ip(addr.ip()) {
                return Err(format!("SSRF blocked: {host} resolves to private IP {}", addr.ip()));
            }
        }
    }

    let blocked_hosts = [
        "metadata.google.internal",
        "169.254.169.254",
        "metadata.azure.com",
    ];
    if blocked_hosts.iter().any(|&h| host == h) {
        return Err(format!("SSRF blocked: cloud metadata endpoint {host}"));
    }

    Ok(())
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

fn parse_ddg_results(html: &str, max: usize) -> Vec<(String, String, String)> {
    let mut results = Vec::new();

    for chunk in html.split("class=\"result__a\"") {
        if results.len() >= max {
            break;
        }
        if !chunk.contains("href=") {
            continue;
        }

        let url = extract_between(chunk, "href=\"", "\"").unwrap_or_default().to_string();
        let actual_url = if url.contains("uddg=") {
            url.split("uddg=")
                .nth(1)
                .and_then(|u| u.split('&').next())
                .map(urldecode)
                .unwrap_or(url)
        } else {
            url
        };

        let title = extract_between(chunk, ">", "<")
            .map(strip_html_tags)
            .unwrap_or_default();

        let snippet = if let Some(snip_start) = chunk.find("class=\"result__snippet\"") {
            let after = &chunk[snip_start..];
            extract_between(after, ">", "<")
                .map(strip_html_tags)
                .unwrap_or_default()
        } else {
            String::new()
        };

        if !actual_url.is_empty() && !title.is_empty() {
            results.push((title, actual_url, snippet));
        }
    }

    results
}

fn extract_between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let start_idx = s.find(start)? + start.len();
    let end_idx = s[start_idx..].find(end)? + start_idx;
    Some(&s[start_idx..end_idx])
}

fn strip_html_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn urldecode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else if ch == '+' {
            result.push(' ');
        } else {
            result.push(ch);
        }
    }
    result
}
