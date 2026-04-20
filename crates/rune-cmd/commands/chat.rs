use anyhow::{Context, Result};
use futures_util::TryStreamExt;
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::io::StreamReader;
use uuid::Uuid;

use crate::cli::ChatArgs;

#[derive(Debug, Deserialize)]
struct InvokeResponse {
    #[allow(dead_code)]
    request_id: Uuid,
    session_id: Uuid,
    output: serde_json::Value,
}

fn invoke_url(gateway: &str, agent: &str) -> Result<Url> {
    let base = gateway.trim_end_matches('/');
    let mut url = Url::parse(base).with_context(|| format!("invalid --gateway URL: {base}"))?;
    url.path_segments_mut()
        .map_err(|_| {
            anyhow::anyhow!(
                "--gateway must be an absolute URL with a path root (e.g. http://localhost:8080)"
            )
        })?
        .push("v1")
        .push("agents")
        .push(agent)
        .push("invoke");
    Ok(url)
}

fn format_output(output: &serde_json::Value) -> String {
    if let Some(s) = output.as_str() {
        return s.to_owned();
    }
    serde_json::to_string_pretty(output).unwrap_or_else(|_| output.to_string())
}

/// Strip `data: ` prefix from one SSE line (for tests and incremental parsing).
fn sse_data_payload(line: &str) -> Option<&str> {
    line.strip_prefix("data: ").map(str::trim)
}

/// Handle one parsed JSON event from gateway invoke SSE. Returns session id from `done` events.
fn handle_invoke_sse_json(v: &Value, saw_token: &mut bool) -> Option<Uuid> {
    use std::io::Write;

    let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match ty {
        "thinking" => {
            if let Some(t) = v.get("text").and_then(|x| x.as_str()) {
                eprintln!("[thinking] {}", t);
            }
        }
        "token" => {
            if let Some(t) = v.get("text").and_then(|x| x.as_str()) {
                *saw_token = true;
                print!("{}", t);
                let _ = std::io::stdout().flush();
            }
        }
        "tool_start" => {
            println!();
            if let Some(n) = v.get("name").and_then(|x| x.as_str()) {
                eprintln!("[tool_start] {}", n);
            }
        }
        "tool_done" => {
            let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("?");
            eprintln!("[tool_done] {} {}", name, v.get("result").unwrap_or(&Value::Null));
        }
        "done" => {
            println!();
            if let Some(s) = v.get("session_id").and_then(|x| x.as_str()) {
                if let Ok(uuid) = Uuid::parse_str(s) {
                    return Some(uuid);
                }
            }
        }
        "error" => {
            if let Some(m) = v.get("message").and_then(|x| x.as_str()) {
                eprintln!("[error] {}", m);
            }
        }
        _ => {}
    }
    None
}

async fn send_invoke_stream(
    client: &reqwest::Client,
    url: Url,
    text: &str,
    session_id: Option<Uuid>,
) -> Result<Option<Uuid>> {
    let mut body = serde_json::json!({
        "input": { "text": text },
        "stream": true
    });
    if let Some(sid) = session_id {
        body["session_id"] = serde_json::json!(sid);
    }

    let resp = client
        .post(url)
        .header("Accept", "text/event-stream")
        .json(&body)
        .send()
        .await
        .with_context(|| {
            "failed to reach gateway — is `rune daemon start` running and `--gateway` correct?"
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        anyhow::bail!("request failed ({status}): {err_body}");
    }

    let byte_stream = resp.bytes_stream().map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::Other, e)
    });
    let reader = StreamReader::new(byte_stream);
    let mut lines = BufReader::new(reader).lines();

    let mut session_from_done: Option<Uuid> = None;
    let mut saw_token = false;

    while let Some(line) = lines.next_line().await? {
        let data = match sse_data_payload(&line) {
            Some(d) if !d.is_empty() => d,
            _ => continue,
        };
        let v: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(sid) = handle_invoke_sse_json(&v, &mut saw_token) {
            session_from_done = Some(sid);
        }
    }

    if saw_token {
        println!();
    }

    Ok(session_from_done)
}

pub async fn exec(args: ChatArgs) -> Result<()> {
    let url = invoke_url(&args.gateway, &args.agent)?;
    let client = reqwest::Client::new();

    println!(
        "Chat with '{}'. Commands: /quit, /exit. Empty lines are ignored.{}",
        args.agent,
        if args.stream {
            " Streaming: token / tool / thinking events on stdout/stderr."
        } else {
            ""
        }
    );

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line_buf = String::new();
    let mut session_id: Option<Uuid> = None;

    loop {
        line_buf.clear();
        print!("> ");
        use std::io::Write;
        std::io::stdout().flush()?;

        let n = reader.read_line(&mut line_buf).await?;
        if n == 0 {
            break;
        }

        let text = line_buf.trim_end_matches(['\r', '\n']).trim();
        if text.is_empty() {
            continue;
        }
        if text == "/quit" || text == "/exit" {
            break;
        }

        if args.stream {
            match send_invoke_stream(&client, url.clone(), text, session_id).await {
                Ok(done_sid) => {
                    if let Some(sid) = done_sid {
                        if session_id.is_none() {
                            println!("(session {})", sid);
                        }
                        session_id = Some(sid);
                    }
                }
                Err(e) => {
                    eprintln!("{e:#}");
                    if e.to_string().contains("404") {
                        eprintln!(
                            "hint: deploy from an artifact (`rune artifact build` then `rune run <name>` or registry) and ensure the gateway is up (`rune daemon start`)."
                        );
                    }
                }
            }
            continue;
        }

        let mut body = serde_json::json!({
            "input": { "text": text },
            "stream": false
        });
        if let Some(sid) = session_id {
            body["session_id"] = serde_json::json!(sid);
        }

        let resp = client
            .post(url.clone())
            .json(&body)
            .send()
            .await
            .with_context(|| {
                "failed to reach gateway — is `rune daemon start` running and `--gateway` correct?"
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            eprintln!("request failed ({status}): {err_body}");
            if status.as_u16() == 404 {
                eprintln!(
                    "hint: deploy from an artifact (`rune artifact build` then `rune run <name>` or registry) and ensure the gateway is up (`rune daemon start`)."
                );
            }
            continue;
        }

        let parsed: InvokeResponse = resp
            .json()
            .await
            .context("invalid JSON from gateway (expected invoke response)")?;

        if session_id.is_none() {
            session_id = Some(parsed.session_id);
            println!("(session {})", parsed.session_id);
        }

        println!("{}", format_output(&parsed.output));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_data_payload_strips_prefix() {
        assert_eq!(
            sse_data_payload(r#"data: {"type":"token"}"#),
            Some(r#"{"type":"token"}"#)
        );
        assert_eq!(sse_data_payload("event: ping"), None);
    }

    #[test]
    fn handle_done_yields_session() {
        let v = serde_json::json!({
            "type": "done",
            "session_id": "550e8400-e29b-41d4-a716-446655440000",
            "request_id": "660e8400-e29b-41d4-a716-446655440001"
        });
        let mut saw = false;
        let sid = handle_invoke_sse_json(&v, &mut saw);
        assert_eq!(
            sid,
            Some(Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap())
        );
    }
}
