use base64::Engine;
use crate::ToolContext;

const MAX_TTS_CHARS: usize = 4096;

fn openai_config(ctx: &ToolContext) -> Result<(String, String), String> {
    let key = ctx.env.openai_api_key.as_ref()
        .map(|k| k.as_str().to_string())
        .ok_or_else(|| "OPENAI_API_KEY not set. Required for media/AI tools.".to_string())?;
    let base = ctx.env.openai_base_url.clone();
    Ok((key, base))
}

fn openai_http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

// ── image-analyze (kept from original) ─────────────────────────────────

pub async fn image_analyze(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let path = if let Some(root) = &ctx.workspace_root {
        root.join(raw_path)
    } else {
        std::path::PathBuf::from(raw_path)
    };

    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|e| format!("Failed to read file: {e}"))?;

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("unknown")
        .to_lowercase();

    let format = match ext.as_str() {
        "jpg" | "jpeg" => "JPEG",
        "png" => "PNG",
        "gif" => "GIF",
        "webp" => "WebP",
        "bmp" => "BMP",
        "svg" => "SVG",
        "tiff" | "tif" => "TIFF",
        "ico" => "ICO",
        other => other,
    };

    Ok(serde_json::json!({
        "path": path.display().to_string(),
        "format": format,
        "file_size_bytes": metadata.len(),
        "note": "Dimension detection requires an image processing library. Install and configure a vision LLM for content analysis."
    }))
}

// ── image-generate (DALL-E) ────────────────────────────────────────────

pub async fn image_generate(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let (key, base) = openai_config(ctx)?;
    let prompt = input["prompt"]
        .as_str()
        .ok_or("Missing 'prompt' parameter")?;
    let size = input["size"].as_str().unwrap_or("1024x1024");

    let body = serde_json::json!({
        "model": "dall-e-3",
        "prompt": prompt,
        "n": 1,
        "size": size,
        "response_format": "b64_json",
    });

    let resp = openai_http()
        .post(format!("{base}/v1/images/generations"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("DALL-E request failed: {e}"))?;

    let status = resp.status();
    let resp_body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse DALL-E response: {e}"))?;

    if !status.is_success() {
        return Err(format!(
            "DALL-E API error ({}): {}",
            status,
            resp_body.get("error").unwrap_or(&resp_body)
        ));
    }

    let b64 = resp_body["data"][0]["b64_json"]
        .as_str()
        .ok_or("No image data in response")?;
    let revised_prompt = resp_body["data"][0]["revised_prompt"]
        .as_str()
        .unwrap_or(prompt);

    let image_bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("Failed to decode image: {e}"))?;

    let filename = format!("generated-{}.png", uuid::Uuid::new_v4());
    let save_path = if let Some(root) = &ctx.workspace_root {
        let dir = root.join(".rune").join("media");
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("Failed to create media dir: {e}"))?;
        dir.join(&filename)
    } else {
        std::path::PathBuf::from(&filename)
    };

    tokio::fs::write(&save_path, &image_bytes)
        .await
        .map_err(|e| format!("Failed to save image: {e}"))?;

    Ok(serde_json::json!({
        "path": save_path.display().to_string(),
        "size": size,
        "revised_prompt": revised_prompt,
        "bytes": image_bytes.len(),
    }))
}

// ── media-describe (GPT-4 Vision) ──────────────────────────────────────

pub async fn media_describe(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let (key, base) = openai_config(ctx)?;
    let raw_path = input["path"]
        .as_str()
        .ok_or("Missing 'path' parameter")?;
    let prompt = input["prompt"]
        .as_str()
        .unwrap_or("Describe this image in detail.");

    let path = if let Some(root) = &ctx.workspace_root {
        root.join(raw_path)
    } else {
        std::path::PathBuf::from(raw_path)
    };

    let image_bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| format!("Failed to read image: {e}"))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&image_bytes);

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_lowercase();
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/png",
    };

    let body = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": prompt },
                {
                    "type": "image_url",
                    "image_url": { "url": format!("data:{mime};base64,{b64}") }
                }
            ]
        }],
        "max_tokens": 1024,
    });

    let resp = openai_http()
        .post(format!("{base}/v1/chat/completions"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Vision request failed: {e}"))?;

    let status = resp.status();
    let resp_body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse vision response: {e}"))?;

    if !status.is_success() {
        return Err(format!(
            "Vision API error ({}): {}",
            status,
            resp_body.get("error").unwrap_or(&resp_body)
        ));
    }

    let description = resp_body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    Ok(serde_json::json!({
        "path": path.display().to_string(),
        "description": description,
        "model": "gpt-4o",
    }))
}

// ── media-transcribe / speech-to-text (Whisper) ────────────────────────

async fn whisper_transcribe(input: &serde_json::Value, ctx: &crate::ToolContext) -> Result<serde_json::Value, String> {
    let (key, base) = openai_config(ctx)?;
    let path = input["path"]
        .as_str()
        .ok_or("Missing 'path' parameter")?;
    let language = input["language"].as_str();

    let file_bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("Failed to read audio file: {e}"))?;

    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio.mp3")
        .to_string();

    let file_part = reqwest::multipart::Part::bytes(file_bytes)
        .file_name(filename)
        .mime_str("application/octet-stream")
        .map_err(|e| format!("Failed to create multipart: {e}"))?;

    let mut form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .text("response_format", "verbose_json")
        .part("file", file_part);

    if let Some(lang) = language {
        form = form.text("language", lang.to_string());
    }

    let resp = openai_http()
        .post(format!("{base}/v1/audio/transcriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Whisper request failed: {e}"))?;

    let status = resp.status();
    let resp_body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Whisper response: {e}"))?;

    if !status.is_success() {
        return Err(format!(
            "Whisper API error ({}): {}",
            status,
            resp_body.get("error").unwrap_or(&resp_body)
        ));
    }

    Ok(serde_json::json!({
        "text": resp_body["text"].as_str().unwrap_or(""),
        "language": resp_body["language"].as_str().unwrap_or(""),
        "duration_seconds": resp_body["duration"],
    }))
}

pub async fn media_transcribe(input: &serde_json::Value, ctx: &crate::ToolContext) -> Result<serde_json::Value, String> {
    whisper_transcribe(input, ctx).await
}

pub async fn speech_to_text(input: &serde_json::Value, ctx: &crate::ToolContext) -> Result<serde_json::Value, String> {
    whisper_transcribe(input, ctx).await
}

// ── text-to-speech (TTS) ──────────────────────────────────────────────

pub async fn text_to_speech(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let (key, base) = openai_config(ctx)?;
    let text = input["text"]
        .as_str()
        .ok_or("Missing 'text' parameter")?;
    let voice = input["voice"].as_str().unwrap_or("alloy");

    if text.len() > MAX_TTS_CHARS {
        return Err(format!(
            "Text too long: {} chars (max {})",
            text.len(),
            MAX_TTS_CHARS
        ));
    }

    let body = serde_json::json!({
        "model": "tts-1",
        "input": text,
        "voice": voice,
        "response_format": "mp3",
    });

    let resp = openai_http()
        .post(format!("{base}/v1/audio/speech"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("TTS request failed: {e}"))?;

    if !resp.status().is_success() {
        let err_body: serde_json::Value = resp
            .json()
            .await
            .unwrap_or(serde_json::json!({"error": "unknown"}));
        return Err(format!(
            "TTS API error: {}",
            err_body.get("error").unwrap_or(&err_body)
        ));
    }

    let audio_bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read TTS audio: {e}"))?;

    let filename = format!("speech-{}.mp3", uuid::Uuid::new_v4());
    let save_path = if let Some(root) = &ctx.workspace_root {
        let dir = root.join(".rune").join("media");
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("Failed to create media dir: {e}"))?;
        dir.join(&filename)
    } else {
        std::path::PathBuf::from(&filename)
    };

    tokio::fs::write(&save_path, &audio_bytes)
        .await
        .map_err(|e| format!("Failed to save audio: {e}"))?;

    Ok(serde_json::json!({
        "path": save_path.display().to_string(),
        "voice": voice,
        "bytes": audio_bytes.len(),
        "format": "mp3",
    }))
}
