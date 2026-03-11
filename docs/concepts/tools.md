# Tools

Tools extend an agent's capabilities. Rune supports several runtimes and a built-in tool library.

## Tool Runtimes

| Runtime | Description |
|---------|-------------|
| `process` | Run a script (Python, Node, etc.) — stdin/stdout JSON protocol |
| `wasm` | WebAssembly module via Wasmtime |
| `container` | Docker container |
| `agent` | Call another agent via A2A (`agent_ref` required) |
| `builtin` | Rune's built-in `rune@*` tools |

## Process Tool Protocol

For `runtime: process` tools (e.g. `.py`, `.js`):

1. Rune writes **one JSON object** to the tool's stdin
2. The tool writes **one JSON object** to stdout and exits
3. stderr is used for diagnostics
4. Non-zero exit code is treated as failure

File extensions map to runtimes: `.py` → `python3`, `.js`/`.mjs`/`.ts` → `node`

## Built-in Tools (`rune@*`)

All built-in tools use the `rune@` prefix. The LLM sees `rune__` (double underscore) in prompts.

| Category | Tools |
|----------|-------|
| Filesystem | `file-read`, `file-write`, `file-list`, `apply-patch` |
| Web | `web-search`, `web-fetch` |
| Shell | `shell-exec` |
| System | `system-time`, `location-get` |
| Memory | `memory-list`, `memory-store`, `memory-recall` |
| Knowledge graph | `knowledge-add-entity`, `knowledge-add-relation`, `knowledge-query` |
| Task queue | `task-post`, `task-claim`, `task-complete`, `task-list`, `event-publish` |
| Scheduling | `schedule-create`, `schedule-list`, `schedule-delete`, `cron-create`, `cron-list`, `cron-cancel` |
| Process | `process-start`, `process-poll`, `process-write`, `process-kill`, `process-list` |
| Media | `image-analyze`, `image-generate`, `media-describe`, `media-transcribe`, `text-to-speech`, `speech-to-text` |
| Agent | `agent-send`, `agent-list`, `agent-find`, `agent-spawn`, `agent-kill`, `a2a-discover`, `a2a-send` |
| Docker | `docker-exec` |
| Browser | `browser-navigate`, `browser-click`, `browser-type`, `browser-screenshot`, `browser-read-page`, `browser-close`, `browser-scroll`, `browser-wait`, `browser-run-js`, `browser-back` |
| Channel | `channel-send` |
| Canvas | `canvas-present` |

## Tool Descriptor

Each custom tool has a YAML descriptor (e.g. `tools/my_tool.yaml`):

```yaml
name: my_tool
version: 0.1.0
runtime: process
module: tools/my_tool.py
timeout_ms: 5000
capabilities: [filesystem]  # or [network], []
retry_policy:
  max_attempts: 2
```

See [tools.yaml reference](../reference/tools-yaml.md) for full options.

## Agent Tools

When `runtime: agent`, the tool calls another agent via A2A. Set `agent_ref`:

- `local://agent-name` — agent in the same runtime
- `http://host:port/a2a/agent-name` — remote A2A endpoint
