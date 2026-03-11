use rune_env::PlatformEnv;

pub async fn channel_send(input: &serde_json::Value, env: &PlatformEnv) -> Result<serde_json::Value, String> {
    let channel = input["channel"]
        .as_str()
        .ok_or("Missing 'channel' parameter")?;
    let recipient = input["recipient"]
        .as_str()
        .ok_or("Missing 'recipient' parameter")?;
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter")?;

    match channel {
        "slack" => send_slack(recipient, message, env).await,
        "telegram" => send_telegram(recipient, message, env).await,
        "discord" => send_discord(message, env).await,
        "email" => send_email(recipient, message, env).await,
        "webhook" => send_webhook(recipient, message, env).await,
        other => Err(format!(
            "Unknown channel '{other}'. Supported: slack, telegram, discord, email, webhook"
        )),
    }
}

async fn send_slack(channel: &str, message: &str, env: &PlatformEnv) -> Result<serde_json::Value, String> {
    let webhook_url = env.slack_webhook_url.as_deref()
        .ok_or("RUNE_SLACK_WEBHOOK_URL not set")?;

    let body = serde_json::json!({
        "channel": channel,
        "text": message,
    });

    let resp = reqwest::Client::new()
        .post(webhook_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Slack request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Slack error ({status}): {text}"));
    }

    Ok(serde_json::json!({
        "sent": true,
        "channel": "slack",
        "recipient": channel,
    }))
}

async fn send_telegram(chat_id: &str, message: &str, env: &PlatformEnv) -> Result<serde_json::Value, String> {
    let token = env.telegram_bot_token.as_ref()
        .ok_or("RUNE_TELEGRAM_BOT_TOKEN not set")?;

    let url = format!("https://api.telegram.org/bot{}/sendMessage", token.as_str());
    let body = serde_json::json!({
        "chat_id": chat_id,
        "text": message,
        "parse_mode": "Markdown",
    });

    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Telegram request failed: {e}"))?;

    let status = resp.status();
    let resp_body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Telegram response: {e}"))?;

    if !status.is_success() || resp_body["ok"].as_bool() != Some(true) {
        return Err(format!(
            "Telegram error: {}",
            resp_body.get("description").unwrap_or(&resp_body)
        ));
    }

    Ok(serde_json::json!({
        "sent": true,
        "channel": "telegram",
        "message_id": resp_body["result"]["message_id"],
        "chat_id": chat_id,
    }))
}

async fn send_discord(message: &str, env: &PlatformEnv) -> Result<serde_json::Value, String> {
    let webhook_url = env.discord_webhook_url.as_deref()
        .ok_or("RUNE_DISCORD_WEBHOOK_URL not set")?;

    let body = serde_json::json!({ "content": message });

    let resp = reqwest::Client::new()
        .post(webhook_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Discord request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() && status.as_u16() != 204 {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Discord error ({status}): {text}"));
    }

    Ok(serde_json::json!({
        "sent": true,
        "channel": "discord",
    }))
}

async fn send_email(recipient: &str, message: &str, env: &PlatformEnv) -> Result<serde_json::Value, String> {
    use lettre::message::header::ContentType;
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

    let host = env.smtp_host.as_deref().ok_or("RUNE_SMTP_HOST not set")?;
    let user = env.smtp_user.as_deref().ok_or("RUNE_SMTP_USER not set")?;
    let pass = env.smtp_pass.as_ref().ok_or("RUNE_SMTP_PASS not set")?;
    let from = env.smtp_from.as_deref().unwrap_or(user);

    let email = Message::builder()
        .from(from.parse().map_err(|e| format!("Invalid from address: {e}"))?)
        .to(recipient
            .parse()
            .map_err(|e| format!("Invalid recipient address: {e}"))?)
        .subject("Message from Rune Agent")
        .header(ContentType::TEXT_PLAIN)
        .body(message.to_string())
        .map_err(|e| format!("Failed to build email: {e}"))?;

    let creds = Credentials::new(user.to_string(), pass.as_str().to_string());
    let mailer = AsyncSmtpTransport::<Tokio1Executor>::relay(host)
        .map_err(|e| format!("SMTP relay error: {e}"))?
        .credentials(creds)
        .build();

    mailer
        .send(email)
        .await
        .map_err(|e| format!("Failed to send email: {e}"))?;

    Ok(serde_json::json!({
        "sent": true,
        "channel": "email",
        "recipient": recipient,
    }))
}

async fn send_webhook(url: &str, message: &str, env: &PlatformEnv) -> Result<serde_json::Value, String> {
    let webhook_url = if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        env.outbound_webhook_url.clone().ok_or(
            "Provide a full URL as recipient, or set RUNE_WEBHOOK_URL"
        )?
    };

    let body = serde_json::json!({
        "message": message,
        "source": "rune",
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    let resp = reqwest::Client::new()
        .post(&webhook_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Webhook request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Webhook error ({status}): {text}"));
    }

    Ok(serde_json::json!({
        "sent": true,
        "channel": "webhook",
        "url": webhook_url,
        "status": status.as_u16(),
    }))
}
