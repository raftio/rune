# Runefile — `runtime:` Section Reference

The `runtime:` section in `Runefile` configures concurrency, health probing, timeouts, and resource limits.

## Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `concurrency_limit` | number | `10` | Max concurrent requests per agent replica |
| `health_probe` | object | see below | Health check config |
| `health_probe.path` | string | `"/health"` | HTTP path for health |
| `health_probe.interval_ms` | number | `5000` | Probe interval in ms |
| `startup_timeout_ms` | number | `10000` | Timeout for replica startup |
| `request_timeout_ms` | number | `30000` | Per-request timeout |
| `streaming_enabled` | boolean | `true` | Enable SSE streaming for invoke |
| `checkpoint_policy` | enum | `on_finish` | `on_finish` / `never` / `each_step` |
| `resource_profile` | enum | `small` | `small` / `medium` / `large` |

## Example

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
