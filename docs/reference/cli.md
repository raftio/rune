# CLI Reference

All `rune` commands and options.

## Agent

Manage deployed agents.

```bash
rune agent ls [--control-plane http://localhost:8081]
rune agent inspect <name> [--ns <namespace>] [--alias <alias>] [--control-plane ...]
rune agent sessions <name> [--ns <namespace>] [--alias <alias>] [--control-plane ...] [--database-url ...]
rune agent stop <name> [--ns <namespace>] [--alias <alias>] [--control-plane ...]
rune agent rm <name> [--ns <namespace>] [--alias <alias>] [--force] [--control-plane ...]
```

- `ls` — List all deployed agents
- `inspect` — Show deployment details and replica health for an agent
- `sessions` — View conversation session history for an agent
- `stop` — Scale an agent to 0 replicas (by name)
- `rm` — Remove an agent deployment (by name); `--force` disables FK checks on cascade failure

## Run

Deploy a single agent, or invoke one directly.

**Deploy** — registers the agent with the control plane and starts replicas:

```bash
rune run <agent_spec> [--namespace dev] [--alias stable] [--control-plane http://localhost:8081]
```

- `agent_spec`: Path to a `Runefile`, an agent directory, or `git://repo-url[#subdir]`

**Invoke** — send a single prompt to a running agent and print the response:

```bash
rune run --agent <name> "<prompt>" [--stream] [--session-id <uuid>] [--gateway http://localhost:8080]
```

```bash
# Examples
rune run --agent chat "Hello, what can you do?"
rune run --agent web-researcher "What is Raft consensus?" --stream
rune run --agent memory-agent "My name is Alice." --session-id 550e8400-e29b-41d4-a716-446655440000
```

## Workflow

Run a `workflow.yaml` pipeline across multiple agents.

```bash
rune workflow run --file workflow.yaml --input "your input" [--gateway http://localhost:8080]
```

- `--file` — Path to `workflow.yaml`
- `--input` — Input string, available as `{{ input }}` in step templates

## Status

List deployments and replica health.

```bash
rune status [--control-plane http://localhost:8081]
```

## Sessions

View conversation history for a session or deployment.

```bash
rune sessions <session_id|deployment_id> [--database-url <url>]
```

Accepts either a session ID or a deployment ID. When given a deployment ID, shows the most recent session for that deployment. Use `rune status` or `rune agent ls` to get IDs.

Default `database-url`: `sqlite:~/.rune/rune.db`

## Daemon

Start or stop the Rune server.

```bash
rune daemon start [options]
rune daemon stop [--pid-file /tmp/rune.pid]
rune daemon status [--pid-file /tmp/rune.pid]
```

### Daemon Start Options

| Option | Default | Description |
|--------|---------|-------------|
| `--gateway-addr` | `0.0.0.0:8080` | Gateway HTTP listen |
| `--control-plane-addr` | `0.0.0.0:8081` | Control plane HTTP listen |
| `--database-url` | `sqlite:~/.rune/rune.db` | SQLite path |
| `--pid-file` | `/tmp/rune.pid` | PID file path |
| `--log-file` | `~/.rune/rune.log` | Daemon log file |
| `--foreground` | — | Don't daemonize |
| `--node-id` | — | Raft node ID (cluster mode) |
| `--raft-addr` | `0.0.0.0:9000` | Raft gRPC listen |
| `--peers` | — | Comma-separated `ID@HOST:PORT` |
| `--backend` | `wasm` | `wasm` / `docker` / `kubernetes` |
| `--k8s-namespace` | `rune` | K8s namespace (when backend=kubernetes) |

## Stop

Scale a deployment to 0 replicas (by deployment ID).

```bash
rune stop <deployment_id> [--control-plane http://localhost:8081]
```

Use `rune agent stop <name>` to stop by agent name instead.

## Rm

Remove a deployment entirely (by deployment ID).

```bash
rune rm <deployment_id> [--force] [--control-plane http://localhost:8081]
```

- `--force` — Force delete when normal cascade fails; disables foreign key checks (may leave orphan rows).

Use `rune agent rm <name>` to remove by agent name instead.

## Compose

Deploy and manage multiple agents from a compose file.

```bash
rune compose up [-f rune-compose.yaml] [--control-plane http://localhost:8081]
rune compose down [-f rune-compose.yaml] [--control-plane http://localhost:8081]
rune compose ps [-f rune-compose.yaml] [--control-plane http://localhost:8081]
```

Default compose file name: `rune-compose.yaml` (also accepts `rune-compose.yml`).

## Cluster

Manage Raft cluster membership.

```bash
rune cluster init [--addr http://localhost:9000]
rune cluster add-learner <node_id> <node_addr> [--leader-addr http://localhost:9000]
rune cluster change-membership <member_id,...> [--leader-addr http://localhost:9000]
rune cluster status [--addr http://localhost:9000]
```

## Debug

Tail daemon logs or restart the daemon with a new log level.

```bash
rune debug [--restart] [--log-level info] [--pid-file /tmp/rune.pid] [--log-file ~/.rune/rune.log]
```

- `--restart` — Send SIGTERM to the current daemon, then restart it before tailing
- `--log-level` — Log level for the restarted daemon (`trace` / `debug` / `info` / `warn` / `error`)
