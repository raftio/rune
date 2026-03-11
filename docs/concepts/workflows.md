# Workflows

A **workflow** orchestrates multiple agents in a DAG (directed acyclic graph). Each step calls an agent via A2A and can use outputs from previous steps.

## Workflow File

`workflow.yaml` is optional. When present, the workflow executor runs the DAG instead of a single-agent invoke.

```yaml
name: my-workflow
version: 0.1.0
timeout_ms: 120000
steps:
  - id: router
    agent_ref: local://router-agent
    input_template: "{{ input }}"
    depends_on: []
  - id: worker
    agent_ref: local://worker-agent
    input_template: "{{ steps.router.output }}"
    depends_on: [router]
output_step: "{{ steps.worker.output }}"
```

## Step Fields

| Field | Description |
|-------|-------------|
| `id` | Unique step identifier |
| `agent_ref` | A2A endpoint: `local://agent-name` or full URL |
| `input_template` | Template with `{{ input }}` and `{{ steps.<id>.output }}` |
| `depends_on` | Step IDs that must complete first |
| `condition` | Optional; skip step if expression is false |
| `timeout_ms` | Per-step timeout override |

## Template Syntax

- `{{ input }}` — Original workflow input
- `{{ steps.<id>.output }}` — Output of a previous step

## Execution

1. Topological sort of steps (cycle detection)
2. Steps grouped into "waves" — when all deps are done
3. Each wave runs in parallel via `tokio::spawn`
4. Each step calls its `agent_ref` via A2A
5. Final output uses `output_step` template
