# First Agent

Create an agent from scratch using a `Runefile`. We'll build an assistant with one custom tool.

## 1. Create the Directory

```bash
mkdir my-agent && cd my-agent
```

## 2. Write the Runefile

Create `Runefile`:

```yaml
name: my-first-agent
version: 0.1.0
instructions: |
  You are a helpful assistant. When asked about current time,
  use the get_time tool.
default_model: default
toolset:
  - get_time
memory_profile: minimal
max_steps: 10
timeout_ms: 30000

runtime:
  concurrency_limit: 3
  health_probe:
    path: /health
    interval_ms: 5000
  startup_timeout_ms: 10000
  request_timeout_ms: 30000
  streaming_enabled: true
  checkpoint_policy: on_finish
  resource_profile: small

models:
  providers:
    - anthropic   # set ANTHROPIC_API_KEY
    - openai      # set OPENAI_API_KEY as fallback
  model_mapping:
    default: claude-sonnet-4-6
  fallback_policy: next_provider
  token_budget: 100000
  safety_policy: standard
```

Set at least one provider's API key before deploying:

```bash
export ANTHROPIC_API_KEY=sk-ant-...   # Claude
# or
export OPENAI_API_KEY=sk-...          # GPT-4o
# or
export GEMINI_API_KEY=...             # Gemini
```

## 3. Add a Custom Tool

Create `tools/get_time.yaml`:

```yaml
name: get_time
version: 0.1.0
runtime: process
module: tools/get_time.py
timeout_ms: 5000
capabilities: []
```

Create `tools/get_time.py`:

```python
#!/usr/bin/env python3
import json
import datetime
from sys import stdin, stdout

def main():
    data = json.load(stdin)
    result = {"current_time": datetime.datetime.utcnow().isoformat() + "Z"}
    stdout.write(json.dumps(result) + "\n")

if __name__ == "__main__":
    main()
```

Make it executable: `chmod +x tools/get_time.py`

**Process tool protocol:** Rune sends one JSON object to stdin; the tool responds with one JSON object on stdout.

> **Tip:** To use a built-in tool instead (no extra files needed), replace `get_time` in `toolset` with `rune@system-time`.

## 4. Deploy

```bash
rune daemon start --foreground &
rune run .
```

## 5. Invoke

```bash
# CLI (simplest)
rune run --agent my-first-agent "What time is it right now?"

# HTTP
curl -X POST http://localhost:8080/v1/agents/my-first-agent/invoke \
  -H "Content-Type: application/json" \
  -d '{"input": "What time is it right now?"}'
```

## Package Layout

```
my-agent/
├── Runefile
└── tools/
    ├── get_time.yaml
    └── get_time.py
```

See the [full spec reference](../spec.md) for all Runefile options and [examples](../examples/index.md) for more complete samples.
