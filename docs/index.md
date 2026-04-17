# Rune

**Production-grade AI agent runtime** written in Rust. Rune handles the full lifecycle of AI agents: packaging, deploying, scheduling, executing, networking, and running them at scale — across Docker, Kubernetes, and WASM backends.

## Highlights

- **Multi-provider LLM** — Anthropic, OpenAI, Google Gemini, GitHub Copilot, with automatic fallback
- **MCP support** — Agents use external MCP servers as tools; Rune can expose deployed agents as an MCP server at `POST /mcp`
- **Multi-backend** — Run agents as WASM modules, Docker containers, or Kubernetes workloads
- **Agent-to-Agent (A2A)** — Agents can call each other via the Google A2A protocol over JSON-RPC
- **40+ channels** — Integrate with Slack, Telegram, Discord, email, and many more messaging platforms
- **Built-in tools** — Filesystem, web search, shell, memory, knowledge graph, browser automation, scheduling, task queue, and more
- **Workflows** — DAG-based multi-agent pipelines with parallel execution and conditional steps
- **Compose** — Deploy multiple agents together with dependency ordering
- **Clustering** — Optional Raft consensus for multi-node high-availability deployments

## How agents are organized

Each agent is defined by a `Runefile` — a single YAML file combining identity, runtime, and model configuration:

```
my-agent/
├── Runefile        # identity, instructions, runtime, models
└── tools/          # optional custom tool implementations
    ├── my_tool.yaml
    └── my_tool.py   # or .js, .wasm, Dockerfile
```

For multi-agent stacks, a `rune-compose.yml` deploys everything together. Example projects live under the `examples/` directory in this repository.

## Where to go next

Installation, quick start, CLI and API details, and architecture notes are documented in the [**README**](https://github.com/raftio/rune/blob/main/README.md) at the repository root.
