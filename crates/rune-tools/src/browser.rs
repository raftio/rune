use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Mutex, oneshot};

use crate::ToolContext;

const MAX_PAGE_TEXT: usize = 50_000;
const DEFAULT_TIMEOUT_MS: u64 = 5_000;
const SCREENSHOT_DIR: &str = ".rune/screenshots";

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    tokio_tungstenite::tungstenite::Message,
>;

// ── BrowserManager ─────────────────────────────────────────────────────

pub struct BrowserManager {
    session: Mutex<Option<BrowserSession>>,
}

impl BrowserManager {
    pub fn new() -> Self {
        Self {
            session: Mutex::new(None),
        }
    }

    async fn cdp(&self) -> Result<Arc<CdpConnection>, String> {
        let mut guard = self.session.lock().await;
        if guard.as_ref().map_or(true, |s| !s.alive()) {
            let session = BrowserSession::launch().await?;
            *guard = Some(session);
        }
        Ok(guard.as_ref().unwrap().cdp.clone())
    }

    async fn close_session(&self) -> Result<(), String> {
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.take() {
            session.close().await;
        }
        Ok(())
    }
}

impl Default for BrowserManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── BrowserSession ─────────────────────────────────────────────────────

struct BrowserSession {
    child: Mutex<tokio::process::Child>,
    cdp: Arc<CdpConnection>,
}

impl BrowserSession {
    async fn launch() -> Result<Self, String> {
        let chrome = find_chrome()?;
        let port = find_free_port().await;

        let child = tokio::process::Command::new(&chrome)
            .args([
                "--headless=new",
                &format!("--remote-debugging-port={port}"),
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-gpu",
                "--disable-extensions",
                "--disable-background-networking",
                "--disable-sync",
                "--disable-translate",
                "--mute-audio",
                "--no-sandbox",
                "about:blank",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to launch Chrome at '{}': {e}", chrome))?;

        let ws_url = wait_for_cdp(port).await?;
        let cdp = CdpConnection::connect(&ws_url).await?;

        Ok(Self {
            child: Mutex::new(child),
            cdp: Arc::new(cdp),
        })
    }

    fn alive(&self) -> bool {
        self.cdp.alive.load(Ordering::Relaxed)
    }

    async fn close(self) {
        let _ = self.cdp.send("Browser.close", serde_json::json!({})).await;
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
    }
}

// ── CDP WebSocket connection ───────────────────────────────────────────

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>>;

struct CdpConnection {
    tx: Mutex<WsSink>,
    pending: PendingMap,
    next_id: AtomicU64,
    alive: AtomicBool,
}

impl CdpConnection {
    async fn connect(ws_url: &str) -> Result<Self, String> {
        let (ws, _) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|e| format!("CDP WebSocket connect failed: {e}"))?;

        let (tx, mut rx) = ws.split();
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));

        let pending_reader = pending.clone();
        let alive_reader = alive.clone();
        tokio::spawn(async move {
            while let Some(msg) = rx.next().await {
                match msg {
                    Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                            if let Some(id) = val.get("id").and_then(|v| v.as_u64()) {
                                let mut map = pending_reader.lock().await;
                                if let Some(sender) = map.remove(&id) {
                                    let _ = sender.send(val);
                                }
                            }
                        }
                    }
                    Err(_) => break,
                    _ => {}
                }
            }
            alive_reader.store(false, Ordering::Relaxed);
        });

        // Extract from Arcs for the struct fields (the reader task holds clones).
        let alive_val = Arc::try_unwrap(alive)
            .unwrap_or_else(|arc| AtomicBool::new(arc.load(Ordering::Relaxed)));

        Ok(Self {
            tx: Mutex::new(tx),
            pending,
            next_id: AtomicU64::new(1),
            alive: alive_val,
        })
    }

    async fn send(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let msg = serde_json::json!({
            "id": id,
            "method": method,
            "params": params,
        });

        let (resp_tx, resp_rx) = oneshot::channel();
        {
            let mut map = self.pending.lock().await;
            map.insert(id, resp_tx);
        }

        {
            let mut tx = self.tx.lock().await;
            tx.send(tokio_tungstenite::tungstenite::Message::Text(
                msg.to_string(),
            ))
            .await
            .map_err(|e| format!("CDP send failed: {e}"))?;
        }

        let result = tokio::time::timeout(std::time::Duration::from_secs(30), resp_rx)
            .await
            .map_err(|_| "CDP response timeout (30s)".to_string())?
            .map_err(|_| "CDP response channel closed".to_string())?;

        if let Some(err) = result.get("error") {
            return Err(format!("CDP error: {}", err));
        }

        Ok(result
            .get("result")
            .cloned()
            .unwrap_or(serde_json::json!({})))
    }
}

// ── Chrome discovery helpers ───────────────────────────────────────────

fn find_chrome() -> Result<String, String> {
    if let Ok(path) = std::env::var("CHROME_PATH") {
        return Ok(path);
    }
    // CHROME_PATH also checked from PlatformEnv at a higher level;
    // keep std::env fallback here since find_chrome() is a standalone helper.

    let candidates = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/snap/bin/chromium",
    ];

    for c in &candidates {
        if std::path::Path::new(c).exists() {
            return Ok(c.to_string());
        }
    }

    if let Ok(output) = std::process::Command::new("which")
        .arg("google-chrome")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(path);
            }
        }
    }

    Err(
        "Chrome/Chromium not found. Set CHROME_PATH or install Google Chrome / Chromium."
            .into(),
    )
}

async fn find_free_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind ephemeral port");
    listener.local_addr().unwrap().port()
}

async fn wait_for_cdp(port: u16) -> Result<String, String> {
    let url = format!("http://127.0.0.1:{port}/json/version");
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);

    loop {
        if tokio::time::Instant::now() > deadline {
            return Err("Chrome CDP did not become ready within 10s".into());
        }

        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| format!("CDP version parse error: {e}"))?;
                let ws_url = body["webSocketDebuggerUrl"]
                    .as_str()
                    .ok_or("No webSocketDebuggerUrl in CDP response")?
                    .to_string();
                return Ok(ws_url);
            }
            _ => {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }
}

// ── Tool implementations ──────────────────────────────────────────────

pub async fn navigate(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let url = input["url"]
        .as_str()
        .ok_or("Missing 'url' parameter")?
        .to_string();

    let cdp = ctx.browser_manager.cdp().await?;
    cdp.send("Page.enable", serde_json::json!({})).await?;
    cdp.send("Page.navigate", serde_json::json!({ "url": url }))
        .await?;

    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    let title_result = cdp
        .send(
            "Runtime.evaluate",
            serde_json::json!({ "expression": "document.title" }),
        )
        .await?;
    let title = title_result["result"]["value"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let url_result = cdp
        .send(
            "Runtime.evaluate",
            serde_json::json!({ "expression": "window.location.href" }),
        )
        .await?;
    let current_url = url_result["result"]["value"]
        .as_str()
        .unwrap_or("")
        .to_string();

    Ok(serde_json::json!({
        "title": title,
        "url": current_url,
        "status": "navigated",
    }))
}

pub async fn click(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let selector = input["selector"]
        .as_str()
        .ok_or("Missing 'selector' parameter")?;

    let cdp = ctx.browser_manager.cdp().await?;
    let js = format!(
        r#"(() => {{
            const el = document.querySelector({sel});
            if (!el) return JSON.stringify({{ error: "Element not found" }});
            el.click();
            return JSON.stringify({{ clicked: true, tag: el.tagName, text: el.textContent.slice(0, 100) }});
        }})()"#,
        sel = serde_json::to_string(selector).unwrap_or_default()
    );
    let result = cdp
        .send(
            "Runtime.evaluate",
            serde_json::json!({ "expression": js, "returnByValue": true }),
        )
        .await?;
    parse_js_result(&result)
}

pub async fn type_text(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let selector = input["selector"]
        .as_str()
        .ok_or("Missing 'selector' parameter")?;
    let text = input["text"]
        .as_str()
        .ok_or("Missing 'text' parameter")?;

    let cdp = ctx.browser_manager.cdp().await?;
    let focus_js = format!(
        r#"(() => {{
            const el = document.querySelector({sel});
            if (!el) return JSON.stringify({{ error: "Element not found" }});
            el.focus();
            return JSON.stringify({{ focused: true }});
        }})()"#,
        sel = serde_json::to_string(selector).unwrap_or_default()
    );
    let focus_result = cdp
        .send(
            "Runtime.evaluate",
            serde_json::json!({ "expression": focus_js, "returnByValue": true }),
        )
        .await?;
    let val = parse_js_result(&focus_result)?;
    if val.get("error").is_some() {
        return Err(val["error"].as_str().unwrap_or("Focus failed").to_string());
    }

    cdp.send("Input.insertText", serde_json::json!({ "text": text }))
        .await?;

    Ok(serde_json::json!({
        "typed": true,
        "selector": selector,
        "length": text.len(),
    }))
}

pub async fn screenshot(ctx: &ToolContext) -> Result<serde_json::Value, String> {
    let cdp = ctx.browser_manager.cdp().await?;
    let result = cdp
        .send(
            "Page.captureScreenshot",
            serde_json::json!({ "format": "png" }),
        )
        .await?;

    let b64 = result["data"].as_str().ok_or("No screenshot data")?;

    let image_bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("Failed to decode screenshot: {e}"))?;

    let filename = format!("screenshot-{}.png", uuid::Uuid::new_v4());
    let save_dir = if let Some(root) = &ctx.workspace_root {
        root.join(SCREENSHOT_DIR)
    } else {
        std::path::PathBuf::from(SCREENSHOT_DIR)
    };
    tokio::fs::create_dir_all(&save_dir)
        .await
        .map_err(|e| format!("Failed to create screenshots dir: {e}"))?;
    let save_path = save_dir.join(&filename);

    tokio::fs::write(&save_path, &image_bytes)
        .await
        .map_err(|e| format!("Failed to save screenshot: {e}"))?;

    Ok(serde_json::json!({
        "path": save_path.display().to_string(),
        "bytes": image_bytes.len(),
        "format": "png",
    }))
}

pub async fn read_page(ctx: &ToolContext) -> Result<serde_json::Value, String> {
    let cdp = ctx.browser_manager.cdp().await?;
    let js = format!(
        r#"(() => {{
            const text = document.body.innerText || "";
            const title = document.title || "";
            const url = window.location.href;
            return JSON.stringify({{
                title: title,
                url: url,
                content: text.slice(0, {max}),
                truncated: text.length > {max}
            }});
        }})()"#,
        max = MAX_PAGE_TEXT
    );
    let result = cdp
        .send(
            "Runtime.evaluate",
            serde_json::json!({ "expression": js, "returnByValue": true }),
        )
        .await?;
    parse_js_result(&result)
}

pub async fn close(ctx: &ToolContext) -> Result<serde_json::Value, String> {
    ctx.browser_manager.close_session().await?;
    Ok(serde_json::json!({ "closed": true }))
}

pub async fn scroll(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let direction = input["direction"].as_str().unwrap_or("down");
    let amount = input["amount"].as_i64().unwrap_or(600);

    let (x, y) = match direction {
        "up" => (0, -amount),
        "down" => (0, amount),
        "left" => (-amount, 0),
        "right" => (amount, 0),
        _ => (0, amount),
    };

    let cdp = ctx.browser_manager.cdp().await?;
    let js = format!("window.scrollBy({x}, {y})");
    cdp.send("Runtime.evaluate", serde_json::json!({ "expression": js }))
        .await?;

    let pos_js = "JSON.stringify({ x: window.scrollX, y: window.scrollY })";
    let result = cdp
        .send(
            "Runtime.evaluate",
            serde_json::json!({ "expression": pos_js, "returnByValue": true }),
        )
        .await?;
    let pos = parse_js_result(&result)?;

    Ok(serde_json::json!({
        "scrolled": true,
        "direction": direction,
        "amount": amount,
        "position": pos,
    }))
}

pub async fn wait(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let selector = input["selector"]
        .as_str()
        .ok_or("Missing 'selector' parameter")?;
    let timeout_ms = input["timeout_ms"].as_u64().unwrap_or(DEFAULT_TIMEOUT_MS);

    let cdp = ctx.browser_manager.cdp().await?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    let check_js = format!(
        "document.querySelector({sel}) !== null",
        sel = serde_json::to_string(selector).unwrap_or_default()
    );

    loop {
        let result = cdp
            .send(
                "Runtime.evaluate",
                serde_json::json!({ "expression": check_js, "returnByValue": true }),
            )
            .await?;

        if result["result"]["value"].as_bool() == Some(true) {
            return Ok(serde_json::json!({
                "found": true,
                "selector": selector,
            }));
        }

        if tokio::time::Instant::now() > deadline {
            return Ok(serde_json::json!({
                "found": false,
                "selector": selector,
                "timeout_ms": timeout_ms,
            }));
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

pub async fn run_js(
    input: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    let expression = input["expression"]
        .as_str()
        .ok_or("Missing 'expression' parameter")?;

    let cdp = ctx.browser_manager.cdp().await?;
    let result = cdp
        .send(
            "Runtime.evaluate",
            serde_json::json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": true,
            }),
        )
        .await?;

    if let Some(ex) = result.get("exceptionDetails") {
        return Err(format!("JS error: {}", ex));
    }

    let value = result
        .get("result")
        .and_then(|r| r.get("value"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    Ok(serde_json::json!({
        "value": value,
        "type": result["result"]["type"].as_str().unwrap_or("undefined"),
    }))
}

pub async fn back(ctx: &ToolContext) -> Result<serde_json::Value, String> {
    let cdp = ctx.browser_manager.cdp().await?;
    let history = cdp
        .send("Page.getNavigationHistory", serde_json::json!({}))
        .await?;

    let current_index = history["currentIndex"].as_i64().unwrap_or(0);
    if current_index <= 0 {
        return Ok(serde_json::json!({
            "navigated": false,
            "reason": "Already at the first page in history",
        }));
    }

    let entries = history["entries"]
        .as_array()
        .ok_or("No navigation entries")?;
    let prev_entry = &entries[(current_index - 1) as usize];
    let entry_id = prev_entry["id"]
        .as_i64()
        .ok_or("No entry ID in history")?;

    cdp.send(
        "Page.navigateToHistoryEntry",
        serde_json::json!({ "entryId": entry_id }),
    )
    .await?;

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let url_result = cdp
        .send(
            "Runtime.evaluate",
            serde_json::json!({ "expression": "window.location.href" }),
        )
        .await?;

    Ok(serde_json::json!({
        "navigated": true,
        "url": url_result["result"]["value"].as_str().unwrap_or(""),
    }))
}

// ── Helpers ────────────────────────────────────────────────────────────

fn parse_js_result(result: &serde_json::Value) -> Result<serde_json::Value, String> {
    if let Some(ex) = result.get("exceptionDetails") {
        return Err(format!("JS error: {}", ex));
    }
    let raw = result["result"]["value"].as_str().unwrap_or("{}");
    serde_json::from_str(raw).map_err(|_| format!("Failed to parse JS result: {raw}"))
}
