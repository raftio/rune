# Memory Agent

A personal assistant that remembers things across sessions using the built-in `rune@memory-*` tools.

**What this example shows:**
- Using `rune@memory-list` to discover what the agent already knows at conversation start
- Using `rune@memory-store` to persist user preferences and details
- Using `rune@memory-recall` to retrieve stored values by key
- `memory_profile: extended` for the largest in-session context

## Run

```bash
cd examples/memory-agent
export ANTHROPIC_API_KEY=<your-key>

rune daemon start --foreground &
rune compose up -f rune-compose.yml
```

Then invoke the agent:

```bash
rune run --agent memory-agent "My name is Alice and I prefer concise answers."
# Agent stores user_name=Alice and user_preferences=concise

rune run --agent memory-agent "Do you remember me?"
# Agent recalls user_name and greets Alice by name
```

## How it works

On every new conversation the agent calls `rune@memory-list` to get all stored keys, then `rune@memory-recall` for relevant ones. When the user shares personal details the agent calls `rune@memory-store` with a consistent descriptive key.

Memory is stored in the SQLite backing store and survives daemon restarts.

## Key naming convention

| Key | Example value |
|-----|--------------|
| `user_name` | `"Alice"` |
| `user_language` | `"Vietnamese"` |
| `user_timezone` | `"Asia/Ho_Chi_Minh"` |
| `user_preferences` | `"concise, no emoji"` |
| `last_topic` | `"deployment strategies"` |
