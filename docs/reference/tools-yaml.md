# tools/*.yaml Reference

Tool descriptors define custom tools in the `tools/` directory. Each `*.yaml` file describes one tool.

## Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | — | Tool name (used in `toolset`) |
| `version` | string | `"0.1.0"` | Tool version |
| `runtime` | enum | `process` | `wasm` / `process` / `container` / `agent` / `mcp` / `builtin` |
| `module` | string | `""` | Path to module (relative to agent dir) |
| `timeout_ms` | number | `5000` | Per-tool timeout |
| `retry_policy` | object | `{max_attempts: 1}` | Retry config |
| `retry_policy.max_attempts` | number | `1` | Max retries on failure |
| `capabilities` | string[] | `[]` | e.g. `[filesystem]`, `[network]` |
| `input_schema_ref` | string | `null` | Optional schema reference |
| `output_schema_ref` | string | `null` | Optional schema reference |
| `agent_ref` | string | `null` | A2A endpoint (required when `runtime: agent`) |
| `max_depth` | number | `null` | Max recursion depth for agent tools (unlimited when not set) |
| `mcp_server` | string | `null` | MCP server name from `mcp_servers:` (required when `runtime: mcp`) |

## Example (Process)

```yaml
name: search_kb
version: 0.1.0
runtime: process
module: tools/search_kb.py
timeout_ms: 8000
retry_policy:
  max_attempts: 2
capabilities:
  - filesystem
```

## Example (Agent)

```yaml
name: delegate
version: 0.1.0
runtime: agent
agent_ref: local://worker-agent
timeout_ms: 30000
max_depth: 3
```

## Example (MCP)

Manual tool descriptor for an MCP server tool. Usually auto-discovered via `mcp_servers:` in the Runefile — use manual descriptors only when you need fine-grained control.

```yaml
name: filesystem/read_file
version: 0.1.0
runtime: mcp
mcp_server: filesystem   # matches a name in mcp_servers:
timeout_ms: 5000
```

## Built-in Tools (`rune@*`)

Built-in tools don't need a descriptor file — list them in `toolset` using the `rune@` prefix. The LLM sees them with `rune__` (double underscore).

| Category | Tool Name | Description |
|----------|-----------|-------------|
| Filesystem | `rune@file-read` | Read a file from the filesystem |
| Filesystem | `rune@file-write` | Write or overwrite a file |
| Filesystem | `rune@file-list` | List files in a directory |
| Filesystem | `rune@apply-patch` | Apply a unified diff patch to files |
| Web | `rune@web-search` | Search the web |
| Web | `rune@web-fetch` | Fetch content from a URL |
| Shell | `rune@shell-exec` | Execute a shell command |
| System | `rune@system-time` | Get current date and time |
| System | `rune@location-get` | Get current location |
| Memory | `rune@memory-list` | List all keys currently stored in agent memory |
| Memory | `rune@memory-store` | Store a key-value pair in agent memory |
| Memory | `rune@memory-recall` | Recall a value from agent memory by key |
| Knowledge | `rune@knowledge-add-entity` | Add a node to the knowledge graph |
| Knowledge | `rune@knowledge-add-relation` | Add an edge to the knowledge graph |
| Knowledge | `rune@knowledge-query` | Query the knowledge graph |
| Tasks | `rune@task-post` | Post a task to the task queue |
| Tasks | `rune@task-claim` | Claim the next available task |
| Tasks | `rune@task-complete` | Mark a task as completed |
| Tasks | `rune@task-list` | List tasks in the queue |
| Tasks | `rune@event-publish` | Publish an event |
| Scheduling | `rune@schedule-create` | Create a one-shot scheduled action |
| Scheduling | `rune@schedule-list` | List scheduled actions |
| Scheduling | `rune@schedule-delete` | Delete a scheduled action |
| Scheduling | `rune@cron-create` | Create a recurring cron job |
| Scheduling | `rune@cron-list` | List cron jobs |
| Scheduling | `rune@cron-cancel` | Cancel a cron job |
| Process | `rune@process-start` | Start a long-running background process |
| Process | `rune@process-poll` | Poll the output of a background process |
| Process | `rune@process-write` | Write to a running process's stdin |
| Process | `rune@process-kill` | Kill a running process |
| Process | `rune@process-list` | List running background processes |
| Media | `rune@image-analyze` | Analyze an image |
| Media | `rune@image-generate` | Generate an image |
| Media | `rune@media-describe` | Describe a media file |
| Media | `rune@media-transcribe` | Transcribe audio/video to text |
| Media | `rune@text-to-speech` | Convert text to speech audio |
| Media | `rune@speech-to-text` | Convert speech audio to text |
| Agent | `rune@agent-send` | Send a message to another agent |
| Agent | `rune@agent-list` | List available agents |
| Agent | `rune@agent-find` | Find an agent by name or capability |
| Agent | `rune@agent-spawn` | Spawn a new agent instance |
| Agent | `rune@agent-kill` | Stop a running agent instance |
| Agent | `rune@a2a-discover` | Discover remote A2A agents |
| Agent | `rune@a2a-send` | Send a message to a remote A2A agent |
| Docker | `rune@docker-exec` | Execute a command in a Docker container |
| Browser | `rune@browser-navigate` | Navigate to a URL |
| Browser | `rune@browser-click` | Click an element on the page |
| Browser | `rune@browser-type` | Type text into an input field |
| Browser | `rune@browser-screenshot` | Take a screenshot of the current page |
| Browser | `rune@browser-read-page` | Read the text content of the current page |
| Browser | `rune@browser-close` | Close the browser tab |
| Browser | `rune@browser-scroll` | Scroll the page |
| Browser | `rune@browser-wait` | Wait for an element or condition |
| Browser | `rune@browser-run-js` | Execute JavaScript in the browser |
| Browser | `rune@browser-back` | Navigate back in browser history |
| Channel | `rune@channel-send` | Send a message to a configured channel |
| Canvas | `rune@canvas-present` | Present rich content on the canvas |
