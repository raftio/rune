# HTTP API Reference

Gateway routes exposed by `rune-gateway`. Default gateway base URL: `http://localhost:8080`.

## Invoke

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/agents/:agent_name/invoke` | Invoke an agent. Body: `{ input, session_id?, stream? }`. SSE or JSON response |

## Sessions

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/agents/:agent_name/sessions` | Create a persistent session |
| GET | `/v1/sessions/:session_id` | Get session state |

## Health

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/replicas/:replica_id/health` | Replica health check |

## A2A (Agent-to-Agent)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/.well-known/agent.json` | Global A2A agent card |
| GET | `/a2a/:agent_name/agent-card` | Per-agent A2A card |
| POST | `/a2a/:agent_name` | A2A JSON-RPC endpoint (`message/send`, `message/stream`, `tasks/get`, `tasks/cancel`, `tasks/pushNotificationConfig/set`) |

## Canvas

| Method | Path | Description |
|--------|------|-------------|
| GET | `/canvas/:id` | Serve canvas artifact from `.rune/canvas/{id}.html` |

## Channels

| Method | Path | Description |
|--------|------|-------------|
| POST | `/channels/webhook` | Inbound channel webhook (requires `RUNE_WEBHOOK_SECRET`, `X-Webhook-Signature`) |
| GET | `/channels/status` | Channel adapter statuses |

## Metrics

| Method | Path | Description |
|--------|------|-------------|
| GET | `/metrics` | Prometheus metrics (when configured) |

## Middleware

- **TraceLayer** — OpenTelemetry distributed tracing
- **auth** — JWT validation (`jsonwebtoken`)
- **rate_limit** — Token-bucket rate limiter
