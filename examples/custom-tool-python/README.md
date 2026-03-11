# Custom Python Tool Example

Shows how to add a custom Python tool (`analyse_sentiment`) to a Rune agent.

**What this example shows:**
- `runtime: process` tool calling a Python script via stdin/stdout
- Tool descriptor YAML (`tools/analyse_sentiment.yaml`)
- The process tool protocol: one JSON object in, one JSON object out

## Layout

```
custom-tool-python/
├── Runefile                       # Agent config — references analyse_sentiment
├── rune-compose.yml
└── tools/
    ├── analyse_sentiment.yaml     # Tool descriptor (name, runtime, module path, timeout)
    └── analyse_sentiment.py       # Tool implementation
```

## Run

```bash
cd examples/custom-tool-python
export ANTHROPIC_API_KEY=<your-key>

rune daemon start --foreground &
rune compose up -f rune-compose.yml

rune run --agent sentiment-agent \
  "Analyse: 'I love this product!', 'Absolutely terrible experience.', 'It is what it is.'"
```

## Process tool protocol

Every process tool follows the same contract:

1. Rune writes **one JSON object** to the tool's stdin.
2. The tool writes **one JSON object** to stdout and exits with code 0.
3. stderr is captured for diagnostics.
4. Non-zero exit = tool failure (retried per `retry_policy`).

```
stdin:  {"texts": ["I love this!"]}
stdout: {"results": [{"text": "I love this!", "sentiment": "positive", "score": 0.95}]}
```

## Adding your own tool

1. Create `tools/my_tool.yaml`:

```yaml
name: my_tool
runtime: process
module: tools/my_tool.py   # or .js, .ts
timeout_ms: 5000
```

2. Create `tools/my_tool.py` — read from `sys.stdin`, write to `sys.stdout`.

3. Add `my_tool` to the `toolset:` in your Runefile.

## Supported runtimes

| Runtime | Interpreter |
|---------|-------------|
| `.py` | `python3` |
| `.js`, `.mjs`, `.ts` | `node` |
| `.wasm` | Wasmtime (set `runtime: wasm`) |
| Any binary | `runtime: container` with a Dockerfile |
