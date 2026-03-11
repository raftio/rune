# Models

The `models:` section in the `Runefile` configures which LLM providers and models an agent uses.

## Structure

```yaml
models:
  providers:
    - anthropic
    - openai
  model_mapping:
    default: claude-sonnet-4-6
    fast: claude-haiku-4-5-20251001
  fallback_policy: next_provider
  token_budget: 100000
  safety_policy: standard
```

## Providers

List providers in **order of preference**. Rune tries each in order when `fallback_policy: next_provider`.

| Provider ID | API Key Env Var | Description |
|-------------|-----------------|-------------|
| `anthropic` | `ANTHROPIC_API_KEY` | Claude models (Haiku, Sonnet, Opus) |
| `openai` | `OPENAI_API_KEY` | GPT-4o, GPT-4-turbo, o1, etc. |
| `gemini` | `GEMINI_API_KEY` | Google Gemini (default model: `gemini-2.0-flash`) |
| `copilot` | `GITHUB_TOKEN` or `COPILOT_TOKEN` | GitHub Copilot (OpenAI-compatible endpoint) |
| `claude-code` | `CLAUDE_API_KEY` | Alternative Anthropic key (Claude via `claude-code` alias) |

Auto-detection order when env vars are present: `anthropic` → `claude-code` → `gemini` → `copilot` → `openai`

## Model Mapping

Maps logical aliases (referenced by `default_model`) to physical model IDs:

```yaml
model_mapping:
  default: claude-sonnet-4-6
  fast: claude-haiku-4-5-20251001
  powerful: claude-opus-4-6
```

**Common model IDs:**

| Provider | Model IDs |
|----------|-----------|
| Anthropic | `claude-sonnet-4-6`, `claude-haiku-4-5-20251001`, `claude-opus-4-6` |
| OpenAI | `gpt-4o`, `gpt-4o-mini`, `gpt-4-turbo`, `o1-preview` |
| Gemini | `gemini-2.0-flash`, `gemini-1.5-pro`, `gemini-2.0-pro` |
| Copilot | `gpt-4o` (default for the Copilot endpoint) |

## Fallback Policy

| Value | Behaviour |
|-------|-----------|
| `next_provider` | On failure, try the next provider in the `providers` list |
| `fail` | Fail immediately on the first provider error |

## Token Budget

Maximum tokens (input + output) per request. Default: `100000`.

## Safety Policy

| Value | Description |
|-------|-------------|
| `standard` | Default safety filters |
| `strict` | Stricter content filtering |
| `none` | No filtering |
