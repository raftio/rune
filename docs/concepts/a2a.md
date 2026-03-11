# Agent-to-Agent (A2A)

Rune implements the [Google A2A](https://google.github.io/A2A/) specification for agent-to-agent communication. Agents can invoke each other via JSON-RPC over HTTP.

## Transport

- **Protocol**: JSON-RPC 2.0 over HTTP
- **Methods**: `message/send`, `message/stream`, `tasks/get`, `tasks/cancel`, `tasks/pushNotificationConfig/set`

## Agent Card

Each agent exposes an agent card:

- **Global**: `GET /.well-known/agent.json`
- **Per-agent**: `GET /a2a/:agent_name/agent-card`

The card describes the agent's capabilities and endpoints.

## JSON-RPC Endpoint

`POST /a2a/:agent_name` — Handles A2A JSON-RPC requests.

## Network Policy

Agents can only call each other when they **share at least one network**. The `networks` field in `spec.yaml` (default: `["bridge"]`) controls membership. Enforcement is done by `rune-network`.

## Agent Reference

When invoking another agent (in workflows, agent tools, or built-in `a2a-send`):

- **Local**: `local://agent-name` — same runtime
- **Remote**: `http://host:port/a2a/agent-name` — full URL

## Streaming

`A2aClient` supports both blocking and streaming `message/send` for real-time responses.
