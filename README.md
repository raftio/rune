# Rune

> **Early development** — APIs and features may change. We welcome feedback and contributions.

**Production-grade AI agent runtime** written in Rust. Rune handles the full lifecycle of AI agents: packaging, deploying, scheduling, executing, networking, and running them at scale — across Docker and WASM backends.

## Key Features

- **Multi-provider LLM** — Anthropic, OpenAI, Google Gemini, GitHub Copilot, with automatic fallback
- **MCP support** — agents consume external MCP servers as tools; Rune exposes all deployed agents as an MCP server at `POST /mcp`
- **Multi-backend** — Run agents as WASM modules, Docker containers, or Kubernetes workloads
- **Agent-to-Agent (A2A)** — Agents can call each other via the Google A2A protocol over JSON-RPC
- **Workflows** — DAG-based multi-agent pipelines with parallel execution and conditional steps
- **Compose** — Deploy multiple agents together with dependency ordering
- **Clustering** — Optional Raft consensus for multi-node high-availability deployments

## Project Layout

Each agent is defined by a `Runefile` — a single YAML file combining identity, runtime, and model configuration:

```
my-agent/
├── Runefile        # identity, instructions, runtime, models
└── tools/          # optional custom tool implementations
    ├── my_tool.yaml
    └── my_tool.py   # or .js, .wasm, Dockerfile
```

For multi-agent stacks, a `rune-compose.yml` deploys everything together. See the [`examples/`](examples/) directory for complete samples.
