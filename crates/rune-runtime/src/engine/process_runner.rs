use std::path::Path;
use std::time::Duration;

use rune_spec::ToolDescriptor;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::error::RuntimeError;

const MAX_OUTPUT_BYTES: usize = 1_048_576; // 1 MB

fn resolve_interpreter(module_path: &str) -> Result<&'static str, RuntimeError> {
    let ext = Path::new(module_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    match ext {
        "js" | "mjs" | "ts" => Ok("node"),
        "py" => Ok("python3"),
        other => Err(RuntimeError::ToolExecution(format!(
            "unsupported tool file extension '.{other}' — expected .js, .mjs, .ts, or .py"
        ))),
    }
}

/// Spawn a child process for a `ToolRuntime::Process` tool, pipe JSON
/// input via stdin, and read JSON output from stdout.
///
/// If `extra_env` is provided, those key-value pairs are injected into
/// the child process environment (in addition to the inherited env).
pub async fn run_process(
    descriptor: &ToolDescriptor,
    agent_dir: &Path,
    input: serde_json::Value,
    extra_env: Option<&std::collections::HashMap<String, String>>,
) -> Result<serde_json::Value, RuntimeError> {
    let interpreter = resolve_interpreter(&descriptor.module)?;
    let module_path = agent_dir.join(&descriptor.module);

    if !module_path.exists() {
        return Err(RuntimeError::ToolExecution(format!(
            "tool module not found: {}",
            module_path.display()
        )));
    }

    let timeout = Duration::from_millis(descriptor.timeout_ms);

    let result = tokio::time::timeout(timeout, async {
        let mut cmd = Command::new(interpreter);
        cmd.arg(&module_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        if let Some(env_map) = extra_env {
            cmd.envs(env_map);
        }

        let mut child = cmd.spawn()
            .map_err(|e| RuntimeError::ToolExecution(format!(
                "failed to spawn '{interpreter}': {e}"
            )))?;

        if let Some(mut stdin) = child.stdin.take() {
            let payload = serde_json::to_vec(&input)
                .map_err(|e| RuntimeError::ToolExecution(e.to_string()))?;
            stdin.write_all(&payload).await?;
            drop(stdin);
        }

        let output = child.wait_with_output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let code = output.status.code().unwrap_or(-1);
            return Err(RuntimeError::ToolExecution(format!(
                "tool '{}' exited with code {code}: {stderr}",
                descriptor.name
            )));
        }

        let stdout = &output.stdout;
        if stdout.len() > MAX_OUTPUT_BYTES {
            return Err(RuntimeError::ToolExecution(format!(
                "tool '{}' output exceeds {MAX_OUTPUT_BYTES} bytes ({} bytes)",
                descriptor.name,
                stdout.len()
            )));
        }

        serde_json::from_slice(stdout).map_err(|e| {
            let raw = String::from_utf8_lossy(stdout);
            RuntimeError::ToolExecution(format!(
                "tool '{}' returned invalid JSON: {e}\nraw stdout: {raw}",
                descriptor.name
            ))
        })
    })
    .await;

    match result {
        Ok(inner) => inner,
        Err(_elapsed) => Err(RuntimeError::ToolExecution(format!(
            "tool '{}' timed out after {}ms",
            descriptor.name, descriptor.timeout_ms
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_interpreter() {
        assert_eq!(resolve_interpreter("tools/search.js").unwrap(), "node");
        assert_eq!(resolve_interpreter("tools/search.mjs").unwrap(), "node");
        assert_eq!(resolve_interpreter("tools/search.ts").unwrap(), "node");
        assert_eq!(resolve_interpreter("tools/search.py").unwrap(), "python3");
        assert!(resolve_interpreter("tools/search.rb").is_err());
        assert!(resolve_interpreter("tools/search").is_err());
    }
}
