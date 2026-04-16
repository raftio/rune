# rune-spec

Schema types and parsing for the rune agent specification format.

This crate defines the canonical data structures used to describe a rune agent: its identity, instructions, toolset, model configuration, and runtime constraints. It is the single source of truth for what a valid agent definition looks like — all other crates depend on it rather than duplicating these types.

## Core types

| Type | Description |
|------|-------------|
| `Runefile` | Parsed representation of a `Runefile` YAML file |
| `AgentSpec` | Agent identity, instructions, toolset, skills, and runtime constraints |
| `ModelsSpec` | LLM providers, model mapping, fallback policy, and token budget |
| `ToolDescriptor` | A single tool's name, runtime, module path, and retry/timeout config |
| `ToolRuntime` | Execution backend: `wasm`, `process`, `container`, `agent`, `builtin`, `mcp` |
| `AgentPackage` | Fully loaded agent — spec + models + resolved tool list |
| `SpecError` | Typed errors for IO and parse failures |

## Runefile format

A `Runefile` is a YAML file that merges `AgentSpec` and `ModelsSpec` into a single document:

```yaml
name: my-agent
version: 0.1.0
instructions: |
  You are a helpful assistant.
default_model: default

memory_profile: standard    # minimal | standard | extended (default: minimal)
max_steps: 20               # default: 20
timeout_ms: 30000           # default: 30 000 ms

models:
  providers:
    - anthropic
  model_mapping:
    default: claude-sonnet-4-6
    fast: claude-haiku-4-5
  fallback_policy: next_provider   # next_provider | fail (default: next_provider)
  token_budget: 100000             # default: 100 000
```
