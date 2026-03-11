use crate::ToolContext;

const CANVAS_DIR: &str = ".rune/canvas";

pub async fn canvas_present(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let html = input["html"]
        .as_str()
        .ok_or("Missing 'html' parameter")?;
    let title = input["title"].as_str().unwrap_or("Rune Canvas");

    let full_html = if html.contains("<html") || html.contains("<!DOCTYPE") {
        html.to_string()
    } else {
        format!(
            r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
</head>
<body>
{html}
</body>
</html>"#
        )
    };

    let id = uuid::Uuid::new_v4().to_string();
    let filename = format!("{id}.html");

    let save_dir = if let Some(root) = &ctx.workspace_root {
        root.join(CANVAS_DIR)
    } else {
        std::path::PathBuf::from(CANVAS_DIR)
    };

    tokio::fs::create_dir_all(&save_dir)
        .await
        .map_err(|e| format!("Failed to create canvas dir: {e}"))?;

    let save_path = save_dir.join(&filename);
    tokio::fs::write(&save_path, &full_html)
        .await
        .map_err(|e| format!("Failed to write canvas file: {e}"))?;

    let gateway_base = &ctx.env.gateway_base_url;
    let url = format!("{}/canvas/{id}", gateway_base.trim_end_matches('/'));

    // Best-effort open in default browser
    let _ = open_in_browser(&save_path).await;

    Ok(serde_json::json!({
        "id": id,
        "path": save_path.display().to_string(),
        "url": url,
        "title": title,
    }))
}

async fn open_in_browser(path: &std::path::Path) -> Result<(), String> {
    let path_str = path.display().to_string();
    let cmd = if cfg!(target_os = "macos") {
        ("open", vec![path_str])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C".into(), "start".into(), path_str])
    } else {
        ("xdg-open", vec![path_str])
    };

    tokio::process::Command::new(cmd.0)
        .args(&cmd.1)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to open browser: {e}"))?;

    Ok(())
}
