# Basic Example - Telegram Chat Agent

A simple chat agent integrated with Telegram. The agent uses OpenAI (gpt-4o-mini) for conversational responses.

## Prerequisites

1. Create a bot with [@BotFather](https://t.me/BotFather) and get the bot token.
2. Set `RUNE_TELEGRAM_BOT_TOKEN` and `OPENAI_API_KEY` in your environment.

## Run

```bash
cd examples/basic-example

# Terminal 1: start daemon (loads channels.yaml from CWD)
export RUNE_TELEGRAM_BOT_TOKEN="your-bot-token-from-botfather"
export OPENAI_API_KEY="your-openai-api-key"
rune daemon start --foreground

# Terminal 2: deploy agent
rune compose up -f rune-compose.yml
```

Then message your bot on Telegram. If you receive "No agent configured", use `/agents` to list agents, then `/agent chat` to select the chat agent.

## Cleanup

```bash
rune compose down -f rune-compose.yml
rune daemon stop
```
