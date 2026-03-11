# Runefile — Agent Fields Reference

Root-level fields in the `Runefile` that define the agent's identity, instructions, toolset, and request limits.

> **See also:** [Full Spec Reference](../spec.md) — covers all Runefile sections and `tools/*.yaml`, `workflow.yaml` in one place.

## Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | — | Agent identifier (required). Lowercase, alphanumeric, hyphens |
| `version` | string | — | Semver version (required) |
| `instructions` | string | — | System prompt (required) |
| `default_model` | string | — | Model alias from `models → model_mapping` (required) |
| `toolset` | string[] | `[]` | Tool names (built-in `rune@*` or custom from `tools/`) |
| `memory_profile` | enum | `minimal` | `minimal` / `standard` / `extended` |
| `routing_hints` | object | `{}` | Extra routing metadata |
| `max_steps` | number | `20` | Max LLM turns per request |
| `timeout_ms` | number | `30000` | Request timeout in milliseconds |
| `networks` | string[] | `["bridge"]` | rune-network memberships for A2A |
| `mcp_servers` | array | `[]` | External MCP servers to connect to at load time |

### `mcp_servers` entry fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | — | Logical name; used as tool prefix `{name}/{tool}` |
| `url` | string | — | HTTP URL of the MCP server |
| `headers` | object | `{}` | HTTP headers (supports `${ENV_VAR}` interpolation) |

## Example

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
