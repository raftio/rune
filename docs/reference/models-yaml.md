# Runefile — `models:` Section Reference

The `models:` section in `Runefile` configures LLM providers, model mapping, fallback policy, token budget, and safety.

## Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `providers` | string[] | `[]` | Provider IDs in order of preference |
| `model_mapping` | object | `{}` | Logical name → physical model ID |
| `fallback_policy` | enum | `next_provider` | `next_provider` / `fail` |
| `token_budget` | number | `100000` | Max tokens (input + output) per request |
| `safety_policy` | enum | `standard` | `standard` / `strict` / `none` |

## Supported Providers

| Provider ID | Required Env Var | Notes |
|-------------|-----------------|-------|
| `anthropic` | `ANTHROPIC_API_KEY` | Claude Haiku / Sonnet / Opus |
| `openai` | `OPENAI_API_KEY` | GPT-4o, GPT-4-turbo, o1, etc. |
| `gemini` | `GEMINI_API_KEY` | Default model: `gemini-2.0-flash` (override with `GEMINI_MODEL`) |
| `copilot` | `GITHUB_TOKEN` or `COPILOT_TOKEN` | GitHub Copilot — OpenAI-compatible endpoint |
| `claude-code` | `CLAUDE_API_KEY` | Anthropic via a separate API key alias |

Auto-detection order (when multiple keys are set): `anthropic` → `claude-code` → `gemini` → `copilot` → `openai`

## Example

```yaml
providers:
  - anthropic
  - gemini
  - openai
model_mapping:
  default: claude-sonnet-4-6
  fast: claude-haiku-4-5-20251001
  fallback: gpt-4o-mini
fallback_policy: next_provider
token_budget: 100000
safety_policy: standard
```

## Common Model IDs

| Provider | Model IDs |
|----------|-----------|
| Anthropic | `claude-sonnet-4-6`, `claude-haiku-4-5-20251001`, `claude-opus-4-6` |
| OpenAI | `gpt-4o`, `gpt-4o-mini`, `gpt-4-turbo`, `o1-preview` |
| Gemini | `gemini-2.0-flash`, `gemini-1.5-pro`, `gemini-2.0-pro` |
| Copilot | `gpt-4o` |
