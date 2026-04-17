use anyhow::{Context, Result};
use reqwest::Url;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
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

pub async fn exec(args: ChatArgs) -> Result<()> {
    let url = invoke_url(&args.gateway, &args.agent)?;
    let client = reqwest::Client::new();

    println!(
        "Chat with '{}'. Commands: /quit, /exit. Empty lines are ignored.",
        args.agent
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
                    "hint: ensure the agent is deployed (`rune run …`) and the gateway is up (`rune daemon start`)."
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
