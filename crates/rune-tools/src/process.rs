use crate::ToolContext;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use uuid::Uuid;

const MAX_PROCESSES: usize = 10;
const MAX_BUFFER: usize = 100_000;

struct ManagedProcess {
    id: String,
    command: String,
    child: tokio::process::Child,
    stdout_buf: Arc<Mutex<String>>,
    stderr_buf: Arc<Mutex<String>>,
    stdin: Option<tokio::process::ChildStdin>,
    started_at: chrono::DateTime<chrono::Utc>,
}

pub struct ProcessManager {
    processes: Mutex<HashMap<String, ManagedProcess>>,
}

impl ProcessManager {
    pub fn new() -> Self {
        Self {
            processes: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn start(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let command = input["command"].as_str().ok_or("Missing 'command' parameter")?;
    let args: Vec<String> = input["args"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let mut procs = ctx.process_manager.processes.lock().await;
    if procs.len() >= MAX_PROCESSES {
        return Err(format!("Max {MAX_PROCESSES} processes. Kill one first."));
    }

    let mut cmd = tokio::process::Command::new(command);
    cmd.args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn: {e}"))?;
    let id = Uuid::new_v4().to_string();

    let stdout_buf = Arc::new(Mutex::new(String::new()));
    let stderr_buf = Arc::new(Mutex::new(String::new()));

    // Spawn reader tasks for stdout/stderr
    if let Some(stdout) = child.stdout.take() {
        let buf = stdout_buf.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let mut b = buf.lock().await;
                        if b.len() < MAX_BUFFER {
                            b.push_str(&line);
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    if let Some(stderr) = child.stderr.take() {
        let buf = stderr_buf.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let mut b = buf.lock().await;
                        if b.len() < MAX_BUFFER {
                            b.push_str(&line);
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    let stdin = child.stdin.take();
    let cmd_display = if args.is_empty() {
        command.to_string()
    } else {
        format!("{command} {}", args.join(" "))
    };

    procs.insert(id.clone(), ManagedProcess {
        id: id.clone(),
        command: cmd_display.clone(),
        child,
        stdout_buf,
        stderr_buf,
        stdin,
        started_at: chrono::Utc::now(),
    });

    Ok(serde_json::json!({
        "process_id": id,
        "command": cmd_display,
        "message": "Process started."
    }))
}

pub async fn poll(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let pid = input["process_id"].as_str().ok_or("Missing 'process_id' parameter")?;
    let procs = ctx.process_manager.processes.lock().await;
    let proc = procs.get(pid).ok_or_else(|| format!("Process '{pid}' not found"))?;

    let mut stdout = proc.stdout_buf.lock().await;
    let mut stderr = proc.stderr_buf.lock().await;
    let out = stdout.clone();
    let err = stderr.clone();
    stdout.clear();
    stderr.clear();

    let alive = proc.child.id().is_some();

    Ok(serde_json::json!({
        "process_id": pid,
        "stdout": out,
        "stderr": err,
        "alive": alive,
    }))
}

pub async fn write(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let pid = input["process_id"].as_str().ok_or("Missing 'process_id' parameter")?;
    let data = input["data"].as_str().ok_or("Missing 'data' parameter")?;

    let mut procs = ctx.process_manager.processes.lock().await;
    let proc = procs.get_mut(pid).ok_or_else(|| format!("Process '{pid}' not found"))?;

    let stdin = proc.stdin.as_mut().ok_or("Process stdin not available")?;
    let mut to_write = data.to_string();
    if !to_write.ends_with('\n') {
        to_write.push('\n');
    }
    stdin
        .write_all(to_write.as_bytes())
        .await
        .map_err(|e| format!("Failed to write to stdin: {e}"))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("Failed to flush stdin: {e}"))?;

    Ok(serde_json::json!({
        "process_id": pid,
        "bytes_written": to_write.len(),
    }))
}

pub async fn kill(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let pid = input["process_id"].as_str().ok_or("Missing 'process_id' parameter")?;
    let mut procs = ctx.process_manager.processes.lock().await;
    let mut proc = procs.remove(pid).ok_or_else(|| format!("Process '{pid}' not found"))?;

    let _ = proc.child.kill().await;

    Ok(serde_json::json!({
        "process_id": pid,
        "message": "Process terminated."
    }))
}

pub async fn list(ctx: &ToolContext) -> Result<serde_json::Value, String> {
    let procs = ctx.process_manager.processes.lock().await;
    let items: Vec<serde_json::Value> = procs
        .values()
        .map(|p| {
            let uptime = chrono::Utc::now().signed_duration_since(p.started_at);
            serde_json::json!({
                "process_id": p.id,
                "command": p.command,
                "alive": p.child.id().is_some(),
                "uptime_secs": uptime.num_seconds(),
            })
        })
        .collect();

    Ok(serde_json::json!({ "count": items.len(), "processes": items }))
}
