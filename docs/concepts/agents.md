# Agents

An **agent** in Rune is a deployable unit defined by YAML specs and optional tool implementations. Each agent has an identity, system instructions, a toolset, and runtime configuration.

## Agent Package Layout

An agent is defined by a `Runefile` — a single YAML file that contains all configuration:

```
my-agent/
├── Runefile        # Identity, instructions, runtime, models
└── tools/          # Optional: custom tool implementations
    ├── my_tool.yaml
    └── my_tool.py   # or .js, .wasm, Dockerfile
```

A minimal `Runefile`:

```yaml
name: chat
version: 0.1.0
instructions: You are a helpful assistant.
default_model: default
toolset: []
memory_profile: standard
max_steps: 10
timeout_ms: 30000

runtime:
  concurrency_limit: 10
  health_probe:
    path: /health
    interval_ms: 5000
  streaming_enabled: true
  checkpoint_policy: on_finish

models:
  providers:
    - openai
  model_mapping:
    default: gpt-4o-mini
  fallback_policy: next_provider
  token_budget: 100000
  safety_policy: standard
```

Deploy with `rune run .` (Rune loads `Runefile` from the directory) or `rune run ./Runefile`.

## Identity and Instructions

- **name** — Unique identifier used in URLs and deployment IDs (lowercase, alphanumeric, hyphens)
- **version** — Semver string
- **instructions** — System prompt sent to the LLM on every turn
- **default_model** — Logical model alias from the `models` section of the `Runefile`

## Toolset

The `toolset` lists tool names the agent can use. Each name must match:

- A descriptor in `tools/*.yaml`, or
- A built-in tool like `rune@file-read`, `rune@web-search`

Built-in tools are auto-injected; you don't need a descriptor file for them.

## Memory Profile

| Profile | Description |
|---------|-------------|
| `minimal` | Fewer tokens retained for context |
| `standard` | Balanced context window |
| `extended` | Maximum context retention |

## Networks

Agents can only call each other (via A2A) when they share at least one **network**. Default: `["bridge"]`. Network policy is enforced by `rune-network`.

## Lifecycle

1. **Deploy** — `rune run` or `rune compose up` registers the agent with the control plane
2. **Reconcile** — The runtime ensures desired replica count matches actual (Docker/K8s/WASM)
3. **Invoke** — Clients send requests to `/v1/agents/:name/invoke`
4. **Stop/Remove** — `rune stop` scales to 0; `rune rm` removes the deployment
