# workflow.yaml Reference

Optional DAG of agent steps. Each step calls an agent via A2A. Supports template interpolation for inputs and output.

## Top-Level Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | — | Workflow name |
| `version` | string | — | Workflow version |
| `description` | string | `""` | Optional description |
| `timeout_ms` | number | `120000` | Overall workflow timeout |
| `steps` | array | — | Step definitions (required) |
| `output_step` | string | `null` | Template for final output |

## Step Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `id` | string | — | Unique step ID (required) |
| `agent_ref` | string | — | A2A endpoint: `local://name` or URL |
| `input_template` | string | `"{{ input }}"` | Template with `{{ input }}`, `{{ steps.<id>.output }}` |
| `depends_on` | string[] | `[]` | Step IDs that must complete first |
| `condition` | string | `null` | Skip if false (e.g. `{{ steps.router.output }} == "technical"`) |
| `timeout_ms` | number | `null` | Per-step timeout override |

## Validation

- At least one step required
- Step IDs must be non-empty and unique
- `depends_on` must reference existing steps; no self or cycles
- DAG must be acyclic (topological sort)

## Example

```yaml
name: pipeline
version: 0.1.0
timeout_ms: 120000
steps:
  - id: router
    agent_ref: local://router
    input_template: "{{ input }}"
    depends_on: []
  - id: worker
    agent_ref: local://worker
    input_template: "{{ steps.router.output }}"
    depends_on: [router]
output_step: "{{ steps.worker.output }}"
```
