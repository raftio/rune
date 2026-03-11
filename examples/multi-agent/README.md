# Multi-Agent Example

Three-agent stack: a **router** that classifies requests and delegates to two specialists (**researcher** and **analyst**) via Agent-to-Agent (A2A) calls.

```
User → router → researcher  (web search + knowledge graph)
              → analyst     (data analysis + shell calc)
```

**What this example shows:**
- A2A agent tools (`runtime: agent`, `agent_ref: local://agent-name`)
- Network isolation — all three agents share the `internal` network
- `workflow.yaml` — a sequential pipeline that runs researcher then analyst
- `rune compose up` deploying a multi-agent stack

## Run (router mode)

```bash
cd examples/multi-agent
export ANTHROPIC_API_KEY=<your-key>

rune daemon start --foreground &
rune compose up -f rune-compose.yml

# Ask the router — it decides which specialist(s) to call
rune run --agent router "What is the current inflation rate in Vietnam and what does it mean for the economy?"
```

## Run (workflow mode)

Run the research-then-analyse pipeline directly:

```bash
rune workflow run --file workflow.yaml \
  --input "the impact of Raft consensus on distributed databases"
```

The workflow runs `researcher` first, then feeds its output into `analyst`. The final answer is the analyst's summary.

## Architecture

| Agent | Role | Tools |
|-------|------|-------|
| `router` | Classifies and delegates | `researcher` (A2A), `analyst` (A2A) |
| `researcher` | Web search + knowledge graph | `rune@web-search`, `rune@web-fetch`, `rune@knowledge-*` |
| `analyst` | Data analysis | `rune@shell-exec`, `rune@system-time` |

## Network isolation

All agents join the `internal` network (in addition to `bridge`), which allows A2A calls between them while keeping them isolated from agents on other networks. See the `networks:` field in each Runefile.
