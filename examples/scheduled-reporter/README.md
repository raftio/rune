# Scheduled Reporter Agent

A proactive agent that creates, manages, and executes recurring scheduled tasks.

**What this example shows:**
- `rune@schedule-create` / `rune@schedule-list` / `rune@schedule-delete` — managing cron-like schedules
- `rune@task-post` / `rune@task-claim` / `rune@task-complete` / `rune@task-list` — shared task queue
- `rune@system-time` — time-aware behaviour

## Run

```bash
cd examples/scheduled-reporter
export ANTHROPIC_API_KEY=<your-key>

rune daemon start --foreground &
rune compose up -f rune-compose.yml
```

## Usage

**Create a schedule:**

```bash
rune run --agent scheduled-reporter \
  "Search for top AI news every morning at 8am and post a summary."
# Returns a schedule ID, e.g. sched_abc123
```

**List active schedules:**

```bash
rune run --agent scheduled-reporter "What schedules are running?"
```

**Cancel a schedule:**

```bash
rune run --agent scheduled-reporter "Cancel schedule sched_abc123"
```

**Check the task queue:**

```bash
rune run --agent scheduled-reporter "Show me pending tasks."
```

## How scheduled triggers work

When a schedule fires, the Rune scheduler invokes the agent with a payload like:

```json
{ "trigger": "[SCHEDULED] daily news summary", "schedule_id": "sched_abc123" }
```

The agent detects the `[SCHEDULED]` prefix in its instructions, performs the task (e.g. web search), and calls `rune@task-complete` with the result.

## Schedule syntax

| Natural language | Cron equivalent |
|-----------------|-----------------|
| `every 5 minutes` | `*/5 * * * *` |
| `daily at 9am` | `0 9 * * *` |
| `every Monday at 8am` | `0 8 * * 1` |
| `every hour` | `0 * * * *` |
