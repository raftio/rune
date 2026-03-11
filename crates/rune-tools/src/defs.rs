use crate::BuiltinToolDef;
use serde_json::json;

pub fn all() -> Vec<BuiltinToolDef> {
    vec![
        // ── Filesystem ──────────────────────────────────────────────
        BuiltinToolDef {
            name: "rune@file-read",
            description: "Read the contents of a file. Paths are relative to the agent workspace.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file path to read" }
                },
                "required": ["path"]
            }),
        },
        BuiltinToolDef {
            name: "rune@file-write",
            description: "Write content to a file. Creates parent directories if needed. Paths are relative to the agent workspace.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file path to write to" },
                    "content": { "type": "string", "description": "The content to write" }
                },
                "required": ["path", "content"]
            }),
        },
        BuiltinToolDef {
            name: "rune@file-list",
            description: "List files in a directory. Paths are relative to the agent workspace.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The directory path to list" }
                },
                "required": ["path"]
            }),
        },
        BuiltinToolDef {
            name: "rune@apply-patch",
            description: "Apply a unified diff patch to files. Use for targeted edits instead of full file overwrites.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "patch": { "type": "string", "description": "The patch in unified diff format" }
                },
                "required": ["patch"]
            }),
        },
        // ── Web ─────────────────────────────────────────────────────
        BuiltinToolDef {
            name: "rune@web-search",
            description: "Search the web using DuckDuckGo. Returns structured results with titles, URLs, and snippets. No API key needed.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The search query" },
                    "max_results": { "type": "integer", "description": "Maximum results to return (default: 5, max: 20)" }
                },
                "required": ["query"]
            }),
        },
        BuiltinToolDef {
            name: "rune@web-fetch",
            description: "Fetch a URL with SSRF protection. Supports GET/POST/PUT/PATCH/DELETE. HTML responses are truncated for readability.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The URL to fetch (http/https only)" },
                    "method": { "type": "string", "enum": ["GET","POST","PUT","PATCH","DELETE"], "description": "HTTP method (default: GET)" },
                    "headers": { "type": "object", "description": "Custom HTTP headers as key-value pairs" },
                    "body": { "type": "string", "description": "Request body for POST/PUT/PATCH" }
                },
                "required": ["url"]
            }),
        },
        // ── Shell ───────────────────────────────────────────────────
        BuiltinToolDef {
            name: "rune@shell-exec",
            description: "Execute a shell command and return its output (stdout + stderr).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to execute" },
                    "timeout_seconds": { "type": "integer", "description": "Timeout in seconds (default: 30)" }
                },
                "required": ["command"]
            }),
        },
        // ── System ──────────────────────────────────────────────────
        BuiltinToolDef {
            name: "rune@system-time",
            description: "Get the current date, time, and timezone. Returns ISO 8601 timestamp, Unix epoch, and timezone info.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        BuiltinToolDef {
            name: "rune@location-get",
            description: "Get approximate geographic location based on IP address. Returns city, country, coordinates, and timezone.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        // ── Memory ──────────────────────────────────────────────────
        BuiltinToolDef {
            name: "rune@memory-store",
            description: "Persist a value in shared key-value memory across sessions and restarts. \
                Use consistent, descriptive keys such as 'user_name', 'user_preferences', \
                'user_language', 'last_topic', or 'onboarding_done'. \
                Call this whenever the user shares personal details or preferences worth remembering.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Descriptive storage key, e.g. 'user_name', 'user_preferences'" },
                    "value": { "type": "string", "description": "The value to store (JSON-encode objects/arrays)" }
                },
                "required": ["key", "value"]
            }),
        },
        BuiltinToolDef {
            name: "rune@memory-recall",
            description: "Recall a previously stored value from shared memory by exact key. \
                Call this at the start of a conversation or when the user references something \
                they shared before. Common keys: 'user_name', 'user_preferences', 'user_language', \
                'last_topic'. If unsure what keys exist, use rune@memory-list first.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The exact storage key to recall" }
                },
                "required": ["key"]
            }),
        },
        BuiltinToolDef {
            name: "rune@memory-list",
            description: "List all keys currently stored in shared memory. \
                Use this to discover what has been remembered before calling rune@memory-recall.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        // ── Knowledge Graph ─────────────────────────────────────────
        BuiltinToolDef {
            name: "rune@knowledge-add-entity",
            description: "Add an entity to the knowledge graph. Entities represent people, organizations, projects, concepts, etc.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Display name of the entity" },
                    "entity_type": { "type": "string", "description": "Type: person, organization, project, concept, event, location, document, tool" },
                    "properties": { "type": "object", "description": "Arbitrary key-value properties" }
                },
                "required": ["name", "entity_type"]
            }),
        },
        BuiltinToolDef {
            name: "rune@knowledge-add-relation",
            description: "Add a relation between two entities in the knowledge graph.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Source entity ID or name" },
                    "relation": { "type": "string", "description": "Relation type: works_at, knows_about, related_to, depends_on, etc." },
                    "target": { "type": "string", "description": "Target entity ID or name" },
                    "confidence": { "type": "number", "description": "Confidence score 0.0-1.0 (default: 1.0)" }
                },
                "required": ["source", "relation", "target"]
            }),
        },
        BuiltinToolDef {
            name: "rune@knowledge-query",
            description: "Query the knowledge graph. Filter by source, relation type, and/or target. Returns matching triples.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Filter by source entity (optional)" },
                    "relation": { "type": "string", "description": "Filter by relation type (optional)" },
                    "target": { "type": "string", "description": "Filter by target entity (optional)" }
                }
            }),
        },
        // ── Task Queue ──────────────────────────────────────────────
        BuiltinToolDef {
            name: "rune@task-post",
            description: "Post a task to the shared task queue for another agent to pick up.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short task title" },
                    "description": { "type": "string", "description": "Detailed task description" },
                    "assigned_to": { "type": "string", "description": "Agent name or ID to assign to (optional)" }
                },
                "required": ["title", "description"]
            }),
        },
        BuiltinToolDef {
            name: "rune@task-claim",
            description: "Claim the next available task from the task queue.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "The claiming agent's ID (optional)" }
                }
            }),
        },
        BuiltinToolDef {
            name: "rune@task-complete",
            description: "Mark a previously claimed task as completed with a result.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "The task ID to complete" },
                    "result": { "type": "string", "description": "The result or outcome" }
                },
                "required": ["task_id", "result"]
            }),
        },
        BuiltinToolDef {
            name: "rune@task-list",
            description: "List tasks in the shared queue, optionally filtered by status (pending, in_progress, completed).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "status": { "type": "string", "description": "Filter by status (optional)" }
                }
            }),
        },
        BuiltinToolDef {
            name: "rune@event-publish",
            description: "Publish a custom event that can trigger proactive agents.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "event_type": { "type": "string", "description": "Type identifier for the event" },
                    "payload": { "type": "object", "description": "JSON payload data" }
                },
                "required": ["event_type"]
            }),
        },
        // ── Scheduling ──────────────────────────────────────────────
        BuiltinToolDef {
            name: "rune@schedule-create",
            description: "Schedule a recurring task using natural language or cron syntax. Examples: 'every 5 minutes', 'daily at 9am', '0 */5 * * *'.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "description": { "type": "string", "description": "What this schedule does" },
                    "schedule": { "type": "string", "description": "Natural language or cron expression" }
                },
                "required": ["description", "schedule"]
            }),
        },
        BuiltinToolDef {
            name: "rune@schedule-list",
            description: "List all scheduled tasks with their IDs, descriptions, and schedules.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        BuiltinToolDef {
            name: "rune@schedule-delete",
            description: "Remove a scheduled task by its ID.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The schedule ID to remove" }
                },
                "required": ["id"]
            }),
        },
        BuiltinToolDef {
            name: "rune@cron-create",
            description: "Create a cron job. Supports one-shot (at), recurring (every N seconds), and cron expressions.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Job name" },
                    "schedule": { "type": "string", "description": "Cron expression or natural language" },
                    "action": { "type": "string", "description": "Action description" }
                },
                "required": ["name", "schedule", "action"]
            }),
        },
        BuiltinToolDef {
            name: "rune@cron-list",
            description: "List all cron jobs.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        BuiltinToolDef {
            name: "rune@cron-cancel",
            description: "Cancel a cron job by its ID.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string", "description": "The cron job ID to cancel" }
                },
                "required": ["job_id"]
            }),
        },
        // ── Process Management ──────────────────────────────────────
        BuiltinToolDef {
            name: "rune@process-start",
            description: "Start a long-running process (REPL, server, watcher). Returns a process_id for subsequent operations.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The executable to run" },
                    "args": { "type": "array", "items": { "type": "string" }, "description": "Command-line arguments" }
                },
                "required": ["command"]
            }),
        },
        BuiltinToolDef {
            name: "rune@process-poll",
            description: "Read accumulated stdout/stderr from a running process. Non-blocking.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The process ID from process-start" }
                },
                "required": ["process_id"]
            }),
        },
        BuiltinToolDef {
            name: "rune@process-write",
            description: "Write data to a running process's stdin.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The process ID" },
                    "data": { "type": "string", "description": "Data to write to stdin" }
                },
                "required": ["process_id", "data"]
            }),
        },
        BuiltinToolDef {
            name: "rune@process-kill",
            description: "Terminate a running process and clean up its resources.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The process ID to kill" }
                },
                "required": ["process_id"]
            }),
        },
        BuiltinToolDef {
            name: "rune@process-list",
            description: "List all running processes with their IDs, commands, and status.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        // ── Inter-agent (placeholder) ───────────────────────────────
        BuiltinToolDef {
            name: "rune@agent-send",
            description: "Send a message to another agent and receive their response.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Target agent UUID or name" },
                    "message": { "type": "string", "description": "The message to send" }
                },
                "required": ["agent_id", "message"]
            }),
        },
        BuiltinToolDef {
            name: "rune@agent-list",
            description: "List all currently running agents with their IDs, names, and states.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        BuiltinToolDef {
            name: "rune@agent-find",
            description: "Discover agents by name, tag, tool, or description.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" }
                },
                "required": ["query"]
            }),
        },
        BuiltinToolDef {
            name: "rune@agent-spawn",
            description: "Spawn a new agent from a manifest. Returns the new agent's ID.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "manifest": { "type": "string", "description": "Agent manifest content" }
                },
                "required": ["manifest"]
            }),
        },
        BuiltinToolDef {
            name: "rune@agent-kill",
            description: "Terminate another agent by its ID.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "The agent UUID to kill" }
                },
                "required": ["agent_id"]
            }),
        },
        BuiltinToolDef {
            name: "rune@a2a-discover",
            description: "Discover an external A2A agent by fetching its agent card from a URL.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Base URL of the remote A2A agent" }
                },
                "required": ["url"]
            }),
        },
        BuiltinToolDef {
            name: "rune@a2a-send",
            description: "Send a task/message to an external A2A agent and get the response.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "The message to send" },
                    "agent_url": { "type": "string", "description": "Direct URL of the remote agent" }
                },
                "required": ["message"]
            }),
        },
        // ── Media / AI ──────────────────────────────────────────────
        BuiltinToolDef {
            name: "rune@image-analyze",
            description: "Analyze an image file — returns format, dimensions, and file size.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the image file" }
                },
                "required": ["path"]
            }),
        },
        BuiltinToolDef {
            name: "rune@image-generate",
            description: "Generate images from a text prompt using DALL-E. Requires OPENAI_API_KEY.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Text description of the image to generate" },
                    "size": { "type": "string", "description": "Image size: '1024x1024', '1024x1792', '1792x1024'" }
                },
                "required": ["prompt"]
            }),
        },
        BuiltinToolDef {
            name: "rune@media-describe",
            description: "Describe an image using a vision-capable LLM.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the image file" },
                    "prompt": { "type": "string", "description": "Optional prompt to guide the description" }
                },
                "required": ["path"]
            }),
        },
        BuiltinToolDef {
            name: "rune@media-transcribe",
            description: "Transcribe audio to text using speech-to-text.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the audio file" },
                    "language": { "type": "string", "description": "Optional ISO-639-1 language code" }
                },
                "required": ["path"]
            }),
        },
        BuiltinToolDef {
            name: "rune@text-to-speech",
            description: "Convert text to speech audio.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "The text to convert (max 4096 chars)" },
                    "voice": { "type": "string", "description": "Voice: alloy, echo, fable, onyx, nova, shimmer" }
                },
                "required": ["text"]
            }),
        },
        BuiltinToolDef {
            name: "rune@speech-to-text",
            description: "Transcribe audio to text. Supported: mp3, wav, ogg, flac, m4a, webm.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the audio file" },
                    "language": { "type": "string", "description": "Optional ISO-639-1 language code" }
                },
                "required": ["path"]
            }),
        },
        // ── Docker ──────────────────────────────────────────────────
        BuiltinToolDef {
            name: "rune@docker-exec",
            description: "Execute a command inside a Docker container sandbox. Provide container_id to exec in an existing container, or image to create an ephemeral one.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to execute inside the container" },
                    "image": { "type": "string", "description": "Docker image to use for an ephemeral container (default: ubuntu:latest)" },
                    "container_id": { "type": "string", "description": "ID of an existing running container to exec into" }
                },
                "required": ["command"]
            }),
        },
        // ── Browser ─────────────────────────────────────────────────
        BuiltinToolDef {
            name: "rune@browser-navigate",
            description: "Navigate a browser to a URL. Returns page title and content as markdown.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The URL to navigate to" }
                },
                "required": ["url"]
            }),
        },
        BuiltinToolDef {
            name: "rune@browser-click",
            description: "Click an element on the current browser page by CSS selector.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector or visible text to click" }
                },
                "required": ["selector"]
            }),
        },
        BuiltinToolDef {
            name: "rune@browser-type",
            description: "Type text into an input field on the current browser page.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector for the input field" },
                    "text": { "type": "string", "description": "The text to type" }
                },
                "required": ["selector", "text"]
            }),
        },
        BuiltinToolDef {
            name: "rune@browser-screenshot",
            description: "Take a screenshot of the current browser page.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        BuiltinToolDef {
            name: "rune@browser-read-page",
            description: "Read the current browser page content as markdown.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        BuiltinToolDef {
            name: "rune@browser-close",
            description: "Close the browser session.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        BuiltinToolDef {
            name: "rune@browser-scroll",
            description: "Scroll the browser page.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "direction": { "type": "string", "description": "up, down, left, right (default: down)" },
                    "amount": { "type": "integer", "description": "Pixels to scroll (default: 600)" }
                }
            }),
        },
        BuiltinToolDef {
            name: "rune@browser-wait",
            description: "Wait for a CSS selector to appear on the page.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector to wait for" },
                    "timeout_ms": { "type": "integer", "description": "Max wait time in ms (default: 5000)" }
                },
                "required": ["selector"]
            }),
        },
        BuiltinToolDef {
            name: "rune@browser-run-js",
            description: "Run JavaScript on the current browser page.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "expression": { "type": "string", "description": "JavaScript expression to run" }
                },
                "required": ["expression"]
            }),
        },
        BuiltinToolDef {
            name: "rune@browser-back",
            description: "Go back to the previous page in browser history.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        // ── Channel ─────────────────────────────────────────────────
        BuiltinToolDef {
            name: "rune@channel-send",
            description: "Send a message to a user on a configured channel (email, telegram, slack, etc).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "channel": { "type": "string", "description": "Channel name (email, telegram, slack, discord)" },
                    "recipient": { "type": "string", "description": "Recipient identifier" },
                    "message": { "type": "string", "description": "The message to send" }
                },
                "required": ["channel", "recipient", "message"]
            }),
        },
        // ── Canvas ──────────────────────────────────────────────────
        BuiltinToolDef {
            name: "rune@canvas-present",
            description: "Present an interactive HTML canvas to the user.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "html": { "type": "string", "description": "The HTML content to present" },
                    "title": { "type": "string", "description": "Optional title for the canvas" }
                },
                "required": ["html"]
            }),
        },
    ]
}
