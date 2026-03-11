# Rune

**Production-grade AI agent runtime** written in Rust. Rune handles the full lifecycle of AI agents: packaging, deploying, scheduling, executing, and networking them — across Docker, Kubernetes, and WASM backends.

## Key Features

- **Multi-provider LLM** — Anthropic, OpenAI, Google Gemini, GitHub Copilot, with automatic fallback
- **Multi-backend** — Run agents as WASM modules, Docker containers, or Kubernetes workloads
- **Agent-to-Agent (A2A)** — Agents can call each other via the Google A2A protocol over JSON-RPC
- **40+ Channels** — Integrate with Slack, Telegram, Discord, email, and many more messaging platforms
- **Built-in tools** — Filesystem, web search, shell, memory, knowledge graph, browser automation, scheduling, task queue, and more
- **Workflows** — DAG-based multi-agent pipelines with parallel execution and conditional steps
- **Compose** — Deploy multiple agents together with dependency ordering
- **Clustering** — Optional Raft consensus for multi-node high-availability deployments

## Quick Links

- [Installation](getting-started/installation.md) — Build and install the Rune CLI
- [Quick Start](getting-started/quickstart.md) — Deploy your first agent in minutes
- [First Agent](getting-started/first-agent.md) — Build an agent with a custom tool from scratch
- [Examples](examples/index.md) — Browse all example agents
- [CLI Reference](reference/cli.md) — All `rune` commands
- [HTTP API](reference/api.md) — Gateway endpoints
- [Architecture](architecture/overview.md) — System design and crate map

## Project Layout

Each agent is defined by a `Runefile` — a single YAML file combining identity, runtime, and model configuration:

```
my-agent/
├── Runefile        # identity, instructions, runtime, models
└── tools/          # optional custom tool implementations
    ├── my_tool.yaml
    └── my_tool.py   # or .js, .wasm, Dockerfile
```

For multi-agent stacks, a `rune-compose.yml` deploys everything together. See [examples](examples/index.md) for complete samples.

## Example Agents

| Example | What it shows |
|---------|--------------|
| [basic-example](../examples/basic-example/) | Minimal chat agent — good starting point |
| [memory-agent](../examples/memory-agent/) | Persistent memory across sessions (`rune@memory-*`) |
| [web-researcher](../examples/web-researcher/) | Web search + knowledge graph accumulation |
| [code-assistant](../examples/code-assistant/) | Filesystem + shell + patch — local coding assistant |
| [multi-agent](../examples/multi-agent/) | A2A delegation + workflow DAG across 3 agents |
| [scheduled-reporter](../examples/scheduled-reporter/) | Cron schedules + task queue |
| [custom-tool-python](../examples/custom-tool-python/) | Custom Python process tool |
