pub async fn shell_exec(input: &serde_json::Value) -> Result<serde_json::Value, String> {
    let command = input["command"].as_str().ok_or("Missing 'command' parameter")?;
    let timeout_secs = input["timeout_seconds"].as_u64().unwrap_or(30);

    let (shell, flag) = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };

    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg(flag)
        .arg(command)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        cmd.output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            let exit_code = output.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            const MAX: usize = 100_000;
            let stdout_s = if stdout.len() > MAX {
                format!("{}...\n[truncated, {} bytes total]", &stdout[..MAX], stdout.len())
            } else {
                stdout.to_string()
            };
            let stderr_s = if stderr.len() > MAX {
                format!("{}...\n[truncated, {} bytes total]", &stderr[..MAX], stderr.len())
            } else {
                stderr.to_string()
            };

            Ok(serde_json::json!({
                "exit_code": exit_code,
                "stdout": stdout_s,
                "stderr": stderr_s,
            }))
        }
        Ok(Err(e)) => Err(format!("Failed to execute command: {e}")),
        Err(_) => Err(format!("Command timed out after {timeout_secs}s")),
    }
}
