# Quick Start

Get a Rune agent running in a few minutes.

## 1. Start the Daemon

The Rune daemon runs the gateway (HTTP API) and control plane.

- **Gateway**: `http://localhost:8080` — invoke agents, sessions, A2A
- **Control plane**: `http://localhost:8081` — deploy agents, manage replicas

```bash
rune daemon start --foreground
```

Leave this running in a terminal.

## 2. Set Your API Key

```bash
# Anthropic (Claude) — recommended
export ANTHROPIC_API_KEY=sk-ant-...

# Or OpenAI
export OPENAI_API_KEY=sk-...

# Or Gemini
export GEMINI_API_KEY=...
```

## 3. Deploy an Agent

Deploy the included basic example:

```bash
cd examples/basic-example
rune compose up -f rune-compose.yml
```

Or deploy any agent directory directly:

```bash
rune run ./examples/basic-example
```

## 4. Check Status

```bash
rune status
```

You should see the `chat` deployment and its replica health.

## 5. Invoke the Agent

**Via CLI:**

```bash
rune run --agent chat "Hello, what can you do?"
```

**Via HTTP:**

```bash
curl -X POST http://localhost:8080/v1/agents/chat/invoke \
  -H "Content-Type: application/json" \
  -d '{"input": "Hello, what can you help me with?"}'
```

**Streaming (SSE):**

```bash
curl -X POST http://localhost:8080/v1/agents/chat/invoke \
  -H "Content-Type: application/json" \
  -d '{"input": "Tell me about yourself.", "stream": true}'
```

## 6. Multi-Turn Sessions

Create a session for persistent multi-turn conversations:

```bash
# Create a session
SESSION=$(curl -s -X POST http://localhost:8080/v1/agents/chat/sessions \
  -H "Content-Type: application/json" -d '{}' | jq -r .session_id)

# First turn
curl -X POST http://localhost:8080/v1/agents/chat/invoke \
  -H "Content-Type: application/json" \
  -d "{\"input\": \"My name is Alice.\", \"session_id\": \"$SESSION\"}"

# Second turn — agent remembers the full conversation history
curl -X POST http://localhost:8080/v1/agents/chat/invoke \
  -H "Content-Type: application/json" \
  -d "{\"input\": \"What is my name?\", \"session_id\": \"$SESSION\"}"
```

## 7. Stop and Clean Up

```bash
rune agent stop chat       # scale to 0 replicas
rune agent rm chat         # remove the deployment
rune daemon stop           # stop the daemon
```

## Next Steps

- [First Agent](first-agent.md) — Create an agent from scratch with a custom tool
- [Examples](../examples/index.md) — Browse all example agents (memory, web research, multi-agent, etc.)
- [Compose](../concepts/compose.md) — Deploy multi-agent stacks
- [CLI Reference](../reference/cli.md) — All commands and options
