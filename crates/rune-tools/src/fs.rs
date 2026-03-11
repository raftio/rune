use std::path::{Component, Path, PathBuf};

use crate::ToolContext;

fn validate_path(raw: &str) -> Result<&str, String> {
    for component in Path::new(raw).components() {
        if matches!(component, Component::ParentDir) {
            return Err("Path traversal denied: '..' components are forbidden".into());
        }
    }
    Ok(raw)
}

fn resolve(raw: &str, ctx: &ToolContext) -> Result<PathBuf, String> {
    let _ = validate_path(raw)?;
    if let Some(root) = &ctx.workspace_root {
        Ok(root.join(raw))
    } else {
        Ok(PathBuf::from(raw))
    }
}

pub async fn file_read(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let raw = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let path = resolve(raw, ctx)?;
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("Failed to read file: {e}"))?;
    Ok(serde_json::json!({ "content": content, "path": path.display().to_string() }))
}

pub async fn file_write(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let raw = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let content = input["content"].as_str().ok_or("Missing 'content' parameter")?;
    let path = resolve(raw, ctx)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create directories: {e}"))?;
    }
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| format!("Failed to write file: {e}"))?;
    Ok(serde_json::json!({
        "message": format!("Wrote {} bytes to {}", content.len(), path.display())
    }))
}

pub async fn file_list(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let raw = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let path = resolve(raw, ctx)?;
    let mut entries = tokio::fs::read_dir(&path)
        .await
        .map_err(|e| format!("Failed to list directory: {e}"))?;
    let mut files = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to read entry: {e}"))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.metadata().await.map(|m| m.is_dir()).unwrap_or(false);
        files.push(serde_json::json!({
            "name": if is_dir { format!("{name}/") } else { name },
            "is_dir": is_dir,
        }));
    }
    files.sort_by(|a, b| {
        a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or(""))
    });
    Ok(serde_json::json!({ "entries": files }))
}

pub async fn apply_patch(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let patch_str = input["patch"].as_str().ok_or("Missing 'patch' parameter")?;
    let root = ctx
        .workspace_root
        .as_deref()
        .ok_or("apply-patch requires a workspace root")?;

    let mut applied = 0u32;
    let mut errors: Vec<String> = Vec::new();

    for line in patch_str.lines() {
        if let Some(file_path) = line.strip_prefix("+++ b/").or_else(|| line.strip_prefix("+++ ")) {
            let target = root.join(file_path.trim());
            if target.exists() {
                applied += 1;
            } else {
                errors.push(format!("target file not found: {}", target.display()));
            }
        }
    }

    if errors.is_empty() {
        Ok(serde_json::json!({
            "message": format!("Patch parsed, {applied} file(s) referenced. Full patch application is a work-in-progress."),
            "note": "Currently validates patch structure; full hunk application coming soon."
        }))
    } else {
        Err(format!("Patch errors: {}", errors.join("; ")))
    }
}
