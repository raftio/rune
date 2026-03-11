# rune-compose.yaml Reference

Multi-agent deployment configuration. Defines a project, shared env, and agent entries with dependencies.

## Top-Level Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `version` | string | `"1"` | Compose spec version |
| `project` | string | — | Project name (required) |
| `rune-env` | object | `{}` | Shared env vars for all agents |
| `agents` | object | — | Agent name → entry (required) |

## Agent Entry Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `source` | string | — | Path or `git://repo[#subdir]` (required) |
| `namespace` | string | `"dev"` | Deployment namespace |
| `alias` | string | `"stable"` | Deployment alias |
| `replicas` | number | `1` | Desired replica count |
| `depends_on` | string[] | `[]` | Agent names that must deploy first |
| `env` | object | `{}` | Per-agent env (overrides `rune-env`) |

## Agent Name Rules

1–63 chars, lowercase alphanumeric or `-`, must start and end with alphanumeric.

## Validation

- `project` non-empty
- At least one agent
- `depends_on` references must exist; no self or cycles
- DAG must be acyclic

## Example

```yaml
version: "1"
project: my-stack
rune-env:
  LOG_LEVEL: info
agents:
  chat:
    source: ./chat-agent
    namespace: dev
    alias: stable
    replicas: 2
  analytics:
    source: ./analytics-agent
    depends_on: [chat]
```
