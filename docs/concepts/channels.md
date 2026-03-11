# Channels

Channels connect Rune agents to 40+ messaging platforms. Inbound messages route to agents via the channel bridge; agents can send outbound messages via the `channel-send` tool.

## Supported Platforms

Partial list: Slack, Discord, Telegram, Teams, WhatsApp, Matrix, Mattermost, Signal, Rocket.Chat, Email, IRC, XMPP, Twitch, Reddit, Mastodon, Nostr, Bluesky, LinkedIn, Line, Viber, Webex, Zulip, DingTalk, Feishu, Lark, Guilded, Revolt, Gitter, Keybase, Nextcloud Talk, Ntfy, Gotify, Mumble, Pumble, Twist, Threema, Discourse

## Architecture

- Each platform has a **ChannelAdapter** implementation
- **BridgeManager** owns adapters and exposes a **ChannelBridgeHandle**
- Inbound messages route to the appropriate agent via **AgentRouter**
- Gateway exposes:
  - `POST /channels/webhook` — Inbound HTTP (requires `RUNE_WEBHOOK_SECRET`, `X-Webhook-Signature`)
  - `GET /channels/status` — Channel adapter statuses

## Configuration (channels.yaml)

Channel integrations are configured in a `channels.yaml` file in the agent or project directory:

```yaml
channels:
  - type: telegram
    bot_token: ${RUNE_TELEGRAM_BOT_TOKEN}
  - type: slack
    bot_token: ${RUNE_SLACK_BOT_TOKEN}
    signing_secret: ${RUNE_SLACK_SIGNING_SECRET}
  - type: discord
    bot_token: ${RUNE_DISCORD_BOT_TOKEN}
```

- Values starting with `${...}` are resolved from environment variables at runtime
- The `type` field must match one of the supported platform identifiers (lowercase)
- Multiple channels of different types can be active simultaneously

## Outbound

Agents use the built-in `rune@channel-send` tool to send messages to channels.
