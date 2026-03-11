# Compose

**Compose** deploys multiple agents together from a single file. Use it for multi-agent stacks with dependencies and shared configuration.

## Compose File

Example `rune-compose.yaml`:

```yaml
version: "1"
project: my-rune-stack

rune-env:
  LOG_LEVEL: info

agents:
  chat:
    source: ./rune-python-chat-agent
    alias: stable
    replicas: 1
  analytics:
    source: ./rune-js-analytics-agent
    alias: stable
    replicas: 1
  report:
    source: ./rune-python-report-agent
    alias: stable
    replicas: 1
```

## Agent Entry Fields

| Field | Default | Description |
|-------|---------|-------------|
| `source` | — | Path to agent directory or `git://repo-url[#subdir]` |
| `namespace` | `dev` | Deployment namespace |
| `alias` | `stable` | Deployment alias (e.g. for canary) |
| `replicas` | `1` | Desired replica count |
| `depends_on` | `[]` | Agent names that must be deployed first |
| `env` | `{}` | Per-agent env vars (overrides `rune-env`) |

## Deploy Order

Agents are deployed in **topological order** based on `depends_on`. Cycles are invalid.

## Commands

```bash
rune compose up -f rune-compose.yaml
rune compose down -f rune-compose.yaml
rune compose ps -f rune-compose.yaml
```

## Shared Environment

`rune-env` applies to all agents. Per-agent `env` overrides for that agent.
