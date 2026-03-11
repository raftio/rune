# Rune Spec Reference

A rune agent package is a directory with a `Runefile` that declares everything the runtime needs to load, run, and connect an agent. This document is the authoritative reference for every field.

## Package layout

```
my-agent/
├── Runefile          # required — agent identity, runtime config, model config
├── workflow.yaml     # optional — DAG of agent steps
└── tools/
    ├── search_kb.yaml
    └── delegate.yaml
```

`AgentPackage::load()` reads the `Runefile`, auto-injects built-in `rune@*` tool descriptors, and optionally loads `workflow.yaml` and any `tools/*.yaml` descriptors.

---

## Runefile

The agent package format. Agent identity fields live at the root; `runtime` and `models` are nested sections.

### Example

```yaml
name: chat
version: 0.1.0
instructions: |
  You are a helpful conversational assistant. Reply concisely.
default_model: default
toolset: []
memory_profile: standard
max_steps: 10
timeout_ms: 30000
networks:
  - bridge

runtime:
  concurrency_limit: 10
  health_probe:
    path: /health
    interval_ms: 5000
  startup_timeout_ms: 10000
  request_timeout_ms: 30000
  streaming_enabled: true
  checkpoint_policy: on_finish
  resource_profile: small

models:
  providers:
    - openai
  model_mapping:
    default: gpt-4o-mini
  fallback_policy: next_provider
  token_budget: 100000
  safety_policy: standard
```

---

## Runefile — agent fields

Root-level fields that define the agent's identity, system prompt, toolset, and request limits.

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | **required** | Agent identifier. Lowercase, alphanumeric, hyphens. Used as the routing key in A2A (`local://name`). |
| `version` | string | **required** | Semver version string. |
| `instructions` | string | **required** | System prompt sent to the LLM before every conversation. |
| `default_model` | string | **required** | Logical model alias resolved via `models → model_mapping`. |
| `toolset` | string[] | `[]` | Tool names to enable. Built-in tools use the `rune@` prefix (e.g. `rune@shell-exec`). Custom tools are defined in `tools/*.yaml`. |
| `memory_profile` | enum | `minimal` | Controls how much conversation history is retained. Values: `minimal` / `standard` / `extended`. |
| `routing_hints` | object | `{}` | Arbitrary key-value metadata passed to the scheduler / load-balancer. |
| `max_steps` | number | `20` | Maximum number of LLM turns (tool call + response pairs) per request. |
| `timeout_ms` | number | `30000` | Hard time limit for a single request, in milliseconds. |
| `networks` | string[] | `["bridge"]` | rune-network memberships. Two agents can call each other only when they share at least one network. |

### memory_profile values

| Value | Description |
|-------|-------------|
| `minimal` | Only the current user turn is sent to the LLM. Lowest token usage. |
| `standard` | Recent conversation window is retained (runtime-defined). |
| `extended` | Full conversation history is retained up to the token budget. |

### Example

```yaml
name: my-agent
version: 0.1.0
instructions: |
  You are a helpful assistant. Use tools when needed.
default_model: default
toolset:
  - rune@file-read
  - rune@web-search
  - my_custom_tool
memory_profile: standard
max_steps: 15
timeout_ms: 60000
networks:
  - bridge
```

---

## Runefile — `runtime:` section

Configures the agent replica's concurrency, health probing, timeouts, and resource class.

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `concurrency_limit` | number | `10` | Maximum number of concurrent requests handled by a single replica. |
| `health_probe` | object | see below | HTTP health check configuration. |
| `health_probe.path` | string | `"/health"` | HTTP path polled by the runtime to determine liveness. |
| `health_probe.interval_ms` | number | `5000` | How often (ms) the runtime polls the health endpoint. |
| `startup_timeout_ms` | number | `10000` | Maximum time (ms) allowed for a replica to become healthy after launch. |
| `request_timeout_ms` | number | `30000` | Per-request timeout at the transport layer (independent of the root `timeout_ms`). |
| `streaming_enabled` | boolean | `true` | Enable SSE streaming on the `/invoke` endpoint. |
| `checkpoint_policy` | enum | `on_finish` | When to persist session state to storage. Values: `on_finish` / `never` / `each_step`. |
| `resource_profile` | enum | `small` | CPU/memory class for the replica. Values: `small` / `medium` / `large`. |

### checkpoint_policy values

| Value | Description |
|-------|-------------|
| `on_finish` | Checkpoint once after the full response is produced. |
| `never` | No checkpointing. Fastest, but session state is lost on crash. |
| `each_step` | Checkpoint after every LLM turn. Slowest, most durable. |

### Example

```yaml
concurrency_limit: 5
health_probe:
  path: /health
  interval_ms: 5000
startup_timeout_ms: 15000
request_timeout_ms: 60000
streaming_enabled: true
checkpoint_policy: on_finish
resource_profile: medium
```

---

## Runefile — `models:` section

Configures LLM providers, model aliasing, fallback behavior, token limits, and content safety.

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `providers` | string[] | `[]` | Provider IDs in preference order. The runtime tries them left-to-right when `fallback_policy: next_provider`. |
| `model_mapping` | object | `{}` | Maps logical alias → physical model ID. The root `default_model` must resolve to a key here. |
| `fallback_policy` | enum | `next_provider` | What to do when the primary provider fails. Values: `next_provider` / `fail`. |
| `token_budget` | number | `100000` | Maximum combined input + output tokens allowed per request. |
| `safety_policy` | enum | `standard` | Content filtering strictness. Values: `standard` / `strict` / `none`. |

### Example

```yaml
providers:
  - anthropic
  - openai
model_mapping:
  default: claude-sonnet-4-6
  fast: claude-haiku-4-5
  reasoning: claude-opus-4-6
fallback_policy: next_provider
token_budget: 100000
safety_policy: standard
```

---

## tools/\*.yaml

Each file in `tools/` describes one callable tool. The filename does not matter; the `name` field is used to reference the tool in the Runefile `toolset`.

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | **required** | Tool name as referenced in `toolset`. |
| `version` | string | `"0.1.0"` | Tool version. |
| `runtime` | enum | `process` | Execution backend. Values: `wasm` / `process` / `container` / `agent` / `builtin`. |
| `module` | string | `""` | Path to the executable or WASM module, relative to the agent directory. |
| `timeout_ms` | number | `5000` | Time limit for a single tool invocation. |
| `retry_policy.max_attempts` | number | `1` | Total invocation attempts before returning an error. |
| `capabilities` | string[] | `[]` | Declared sandbox permissions (e.g. `filesystem`, `network`). |
| `input_schema_ref` | string | `null` | Path or URL to a JSON Schema for the tool's input. |
| `output_schema_ref` | string | `null` | Path or URL to a JSON Schema for the tool's output. |
| `agent_ref` | string | `null` | A2A endpoint (`local://agent-name` or full URL). Required when `runtime: agent`. |
| `max_depth` | number | `null` | Maximum A2A recursion depth for agent tools. |

### runtime values

| Value | Description |
|-------|-------------|
| `process` | Launched as a child process (default). |
| `wasm` | Executed in a Wasmtime sandbox. |
| `container` | Launched as a Docker/OCI container. |
| `agent` | Calls another agent via A2A. Requires `agent_ref`. |
| `builtin` | Provided by the rune runtime itself (`rune@*` names). |

### Built-in tools (`rune@`)

Built-in tools are provided by the runtime and do not need a descriptor file. Reference them in `toolset` by name:

| Category | Name | Description |
|----------|------|-------------|
| Filesystem | `rune@file-read` | Read a file from the filesystem |
| Filesystem | `rune@file-write` | Write or overwrite a file |
| Filesystem | `rune@file-list` | List files in a directory |
| Filesystem | `rune@apply-patch` | Apply a unified diff patch to files |
| Web | `rune@web-search` | Search the web |
| Web | `rune@web-fetch` | Fetch content from a URL |
| Shell | `rune@shell-exec` | Execute a shell command |
| System | `rune@system-time` | Get current date and time |
| System | `rune@location-get` | Get current location |
| Memory | `rune@memory-store` | Store a key-value pair in agent memory |
| Memory | `rune@memory-recall` | Recall a value from agent memory |
| Memory | `rune@memory-list` | List all keys currently stored in memory |
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

### Example — process tool

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

### Example — agent tool

```yaml
name: delegate
version: 0.1.0
runtime: agent
agent_ref: local://worker-agent
timeout_ms: 30000
max_depth: 3
```

---

## workflow.yaml

An optional DAG of agent steps. When present, an incoming request is routed through the workflow rather than dispatched directly to the agent.

### Top-level fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | **required** | Workflow name. |
| `version` | string | **required** | Workflow version. |
| `description` | string | `""` | Human-readable description. |
| `timeout_ms` | number | `120000` | Overall workflow deadline in milliseconds. |
| `steps` | array | **required** | List of `WorkflowStep` objects. At least one step is required. |
| `output_step` | string | `null` | Template expression for the final output (e.g. `{{ steps.final.output }}`). |

### Step fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `id` | string | **required** | Unique step identifier within the workflow. |
| `agent_ref` | string | **required** | A2A endpoint: `local://agent-name` or a full HTTP URL. |
| `input_template` | string | `"{{ input }}"` | Template string passed as the step's input. Supports `{{ input }}` (original request) and `{{ steps.<id>.output }}`. |
| `depends_on` | string[] | `[]` | Step IDs that must complete successfully before this step may start. |
| `condition` | string | `null` | Skip expression. The step is skipped when the expression evaluates to false. Format: `{{ steps.<id>.output }} == "value"`. |
| `timeout_ms` | number | `null` | Per-step timeout override. Falls back to the workflow-level `timeout_ms`. |

### Validation rules

- At least one step must be defined.
- Every step `id` must be non-empty and unique within the workflow.
- Every `depends_on` entry must reference an existing step `id`.
- A step cannot depend on itself.
- The dependency graph must be acyclic (validated via topological sort).

### Example

```yaml
name: triage-pipeline
version: 0.1.0
description: Route and handle support tickets
timeout_ms: 120000
steps:
  - id: router
    agent_ref: local://router
    input_template: "{{ input }}"

  - id: technical
    agent_ref: local://technical-support
    input_template: "{{ steps.router.output }}"
    depends_on: [router]
    condition: "{{ steps.router.output }} == technical"

  - id: billing
    agent_ref: local://billing-support
    input_template: "{{ steps.router.output }}"
    depends_on: [router]
    condition: "{{ steps.router.output }} == billing"

output_step: "{{ steps.technical.output }}"
```

---

## Error types

The `rune-spec` crate surfaces three error variants:

| Variant | When raised |
|---------|-------------|
| `SpecError::Io` | File cannot be read (missing, permission denied). |
| `SpecError::Parse` | YAML is malformed or a required field is missing. |
| `SpecError::Validation` | Parsed values violate semantic rules (cycles, empty names, bad references). |

---

## Related reference pages

- [Runefile agent fields](reference/spec-yaml.md)
- [Runefile runtime: section](reference/runtime-yaml.md)
- [Runefile models: section](reference/models-yaml.md)
- [tools/*.yaml](reference/tools-yaml.md)
- [workflow.yaml](reference/workflow-yaml.md)
- [rune-compose.yml](reference/compose-yaml.md)
