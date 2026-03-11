# Example Agents

A collection of example agents covering the full range of Rune capabilities — from simple single-agent chatbots to multi-agent workflows with custom tools.

## Quick start

All examples follow the same pattern:

```bash
cd examples/<example-name>
export ANTHROPIC_API_KEY=<your-key>   # or OPENAI_API_KEY

rune daemon start --foreground &
rune compose up -f rune-compose.yml

rune run --agent <agent-name> "your prompt here"
```

---

## Examples

### [basic-example](../../examples/basic-example/)

A minimal single-file conversational agent. Good starting point for any new agent.

**Tools:** `rune@memory-store`, `rune@memory-recall`, `rune@knowledge-*`

```bash
cd examples/basic-example && rune compose up -f rune-compose.yml
rune run --agent chat "Hello, what can you do?"
```

---

### [memory-agent](../../examples/memory-agent/)

A personal assistant that remembers user preferences and details across sessions and restarts.

**Tools:** `rune@memory-list`, `rune@memory-store`, `rune@memory-recall`

**Key concepts:**
- Call `rune@memory-list` at session start to discover what's stored
- Use consistent key names: `user_name`, `user_preferences`, `user_language`
- Memory persists in SQLite across daemon restarts

```bash
cd examples/memory-agent && rune compose up -f rune-compose.yml
rune run --agent memory-agent "My name is Alice, I prefer concise answers."
rune run --agent memory-agent "Do you remember my name?"
```

---

### [web-researcher](../../examples/web-researcher/)

An agent that searches the web, reads pages, and accumulates knowledge in a graph.

**Tools:** `rune@web-search`, `rune@web-fetch`, `rune@knowledge-add-entity`, `rune@knowledge-add-relation`, `rune@knowledge-query`

**Key concepts:**
- DuckDuckGo search — no API key needed
- SSRF-protected HTTP fetch (HTML auto-truncated)
- Knowledge graph persists across sessions — second query is faster

```bash
cd examples/web-researcher && rune compose up -f rune-compose.yml
rune run --agent web-researcher "What is Raft consensus and who invented it?"
rune run --agent web-researcher "What do you know about Raft?"   # hits knowledge graph
```

---

### [code-assistant](../../examples/code-assistant/)

A local coding assistant with filesystem access and shell execution.

**Tools:** `rune@file-read`, `rune@file-write`, `rune@file-list`, `rune@apply-patch`, `rune@shell-exec`

**Key concepts:**
- `rune@apply-patch` for targeted unified-diff edits (safer than full overwrites)
- `rune@shell-exec` to run tests and linters after changes
- Set `RUNE_WORKSPACE_DIR` to restrict filesystem access

```bash
cd examples/code-assistant
export RUNE_WORKSPACE_DIR=/path/to/your/project
rune compose up -f rune-compose.yml
rune run --agent code-assistant "Add type hints to all functions in src/utils.py and run tests."
```

---

### [multi-agent](../../examples/multi-agent/)

Three-agent stack: a **router** delegates to a **researcher** and an **analyst** via Agent-to-Agent (A2A) calls. Includes a `workflow.yaml` for sequential pipeline execution.

**Tools:** A2A agent tools (`runtime: agent`), `rune@web-search`, `rune@shell-exec`

**Key concepts:**
- `runtime: agent` + `agent_ref: local://agent-name` for A2A delegation
- `networks: [bridge, internal]` for network isolation between agents
- `workflow.yaml` for DAG-based sequential/parallel pipelines

```bash
cd examples/multi-agent && rune compose up -f rune-compose.yml

# Router mode — agent decides which specialist to call
rune run --agent router "What is the current state of LLM benchmarks?"

# Workflow mode — explicit research-then-analyse pipeline
rune workflow run --file workflow.yaml \
  --input "impact of Raft consensus on distributed databases"
```

---

### [scheduled-reporter](../../examples/scheduled-reporter/)

A proactive agent that creates recurring schedules and manages a shared task queue.

**Tools:** `rune@schedule-create`, `rune@schedule-list`, `rune@schedule-delete`, `rune@task-post`, `rune@task-claim`, `rune@task-complete`, `rune@task-list`

**Key concepts:**
- Natural language scheduling: `"every morning at 8am"`, `"every Monday"`
- Cron expressions: `"0 8 * * 1-5"`
- Task queue for async hand-off between agents
- Scheduled triggers arrive with a `[SCHEDULED]` prefix in the input

```bash
cd examples/scheduled-reporter && rune compose up -f rune-compose.yml
rune run --agent scheduled-reporter \
  "Search for top AI news every morning at 8am and post a summary."
rune run --agent scheduled-reporter "List active schedules."
```

---

### [custom-tool-python](../../examples/custom-tool-python/)

A sentiment-analysis agent powered by a custom Python tool.

**Key concepts:**
- `runtime: process` — Rune spawns the script, writes JSON to stdin, reads JSON from stdout
- Tool descriptor YAML defines the tool name, module path, and timeout
- `retry_policy.max_attempts` for transient failures

```bash
cd examples/custom-tool-python && rune compose up -f rune-compose.yml
rune run --agent sentiment-agent \
  "Analyse: 'I love this!', 'Absolutely terrible.', 'It is what it is.'"
```

## Agent package layout

Every agent is a directory (or single Runefile) with this structure:

```
my-agent/
├── Runefile            # Required: identity, instructions, runtime, models
└── tools/              # Optional: custom tool implementations
    ├── my_tool.yaml    # Tool descriptor (name, runtime, module, timeout)
    └── my_tool.py      # Tool implementation (.py, .js, .wasm, ...)
```

A `Runefile` combines all sections in one file:

```yaml
# Identity & behaviour
name: my-agent
version: 0.1.0
instructions: You are a helpful assistant.
default_model: default
toolset:
  - rune@web-search       # built-in tool
  - my_custom_tool        # custom process tool
max_steps: 20
timeout_ms: 60000

# Runtime configuration
runtime:
  concurrency_limit: 10
  streaming_enabled: true
  checkpoint_policy: on_finish   # or: never, on_error
  resource_profile: small        # small | medium | large

# Model configuration
models:
  providers:
    - anthropic
    - openai
  model_mapping:
    default: claude-haiku-4-5-20251001
  fallback_policy: next_provider
  token_budget: 100000
```

---

## Built-in tools reference

| Category | Tool | Description |
|----------|------|-------------|
| **Filesystem** | `rune@file-read` | Read a file from the workspace |
| | `rune@file-write` | Write or create a file |
| | `rune@file-list` | List directory contents |
| | `rune@apply-patch` | Apply a unified diff patch |
| **Web** | `rune@web-search` | DuckDuckGo search (no API key) |
| | `rune@web-fetch` | Fetch a URL (SSRF-protected) |
| **Shell** | `rune@shell-exec` | Run a shell command |
| **System** | `rune@system-time` | Current date/time/timezone |
| | `rune@location-get` | Approximate location via IP |
| **Memory** | `rune@memory-list` | List all stored keys |
| | `rune@memory-store` | Persist a key-value pair |
| | `rune@memory-recall` | Retrieve a stored value |
| **Knowledge** | `rune@knowledge-add-entity` | Add entity to knowledge graph |
| | `rune@knowledge-add-relation` | Add relation between entities |
| | `rune@knowledge-query` | Query the knowledge graph |
| **Task queue** | `rune@task-post` | Post a task to the queue |
| | `rune@task-claim` | Claim the next task |
| | `rune@task-complete` | Complete a claimed task |
| | `rune@task-list` | List tasks by status |
| **Scheduling** | `rune@schedule-create` | Create a recurring schedule |
| | `rune@schedule-list` | List all schedules |
| | `rune@schedule-delete` | Delete a schedule |
| **Events** | `rune@event-publish` | Publish a custom event |
| **A2A** | `runtime: agent` | Delegate to another agent |

---

## Process tool protocol

All `runtime: process` tools use the same contract:

1. Rune writes **one JSON object** to the tool's `stdin`
2. The tool writes **one JSON object** to `stdout` and exits `0`
3. `stderr` is captured for diagnostics and surfaced in logs
4. Non-zero exit = tool failure (retried per `retry_policy`)
5. `timeout_ms` in the descriptor hard-kills the process

**Python template:**
```python
#!/usr/bin/env python3
import json, sys
data = json.load(sys.stdin)
# ... process ...
json.dump({"result": "..."}, sys.stdout)
```

**JavaScript template:**
```js
#!/usr/bin/env node
const data = JSON.parse(require("fs").readFileSync("/dev/stdin", "utf8"));
// ... process ...
process.stdout.write(JSON.stringify({ result: "..." }));
```

---

## Workflow YAML

A `workflow.yaml` defines a DAG of agent calls:

```yaml
name: my-pipeline
version: 0.1.0
timeout_ms: 120000
output_step: "{{ steps.final.output }}"

steps:
  - id: step1
    agent_ref: local://agent-a
    input_template: "{{ input }}"

  - id: step2
    agent_ref: local://agent-b
    input_template: "Summarise: {{ steps.step1.output }}"
    depends_on: [step1]

  - id: final
    agent_ref: local://agent-c
    depends_on: [step1, step2]
    condition: "{{ steps.step1.output }} == ok"
```

Run with: `rune workflow run --file workflow.yaml --input "your input"`

See the [workflow YAML reference](../reference/workflow-yaml.md) for full field documentation.
