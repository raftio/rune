# skillsmp-agent

An agent that integrates with [SkillsMP](https://skillsmp.com) — a marketplace of 66,500+ reusable instruction packages (skills). Demonstrates two complementary patterns for using skills in a Rune agent.

## Overview

| Pattern | When skills are loaded | How to add skills |
|---------|------------------------|-------------------|
| **Static (Option A)** | At agent load time — injected into `instructions` | `npx skills add owner/repo/skill-name` |
| **Dynamic (Option B)** | At runtime via MCP tools | SkillsMP MCP server + `skillsmp/*` tools |

## Prerequisites

- Rune daemon installed and running (`rune daemon start`)
- An LLM API key (`ANTHROPIC_API_KEY` or `OPENAI_API_KEY`)
- Node.js (for `npx skills` and the SkillsMP MCP server)

## Option A — Static skills

Install skills locally before starting the agent. Each `SKILL.md` is read at load time and appended to the agent's instructions automatically.

```bash
# Install skills into skills/<owner>/<repo>/<skill-name>/SKILL.md
npx skills add anthropics/claude-code/frontend-design
npx skills add vercel-labs/agent-skills/find-skills
```

Skills are declared in the Runefile:

```yaml
skills:
  - anthropics/claude-code/frontend-design
  - vercel-labs/agent-skills/find-skills
```

The runtime reads `skills/<owner>/<repo>/<skill-name>/SKILL.md` and appends the content to `instructions`. No restart needed after installing a new skill — changes take effect on the next agent load.

## Option B — Dynamic skills via MCP

The SkillsMP MCP server exposes search, read, and install tools. The agent can discover and apply skills at runtime without redeploying.

```bash
# Get a free API key at https://skillsmp.com
export SKILLSMP_API_KEY=<your-key>

# Start the MCP server on port 3010
npx skillsmp-mcp-server --transport http --port 3010
```

Available MCP tools (prefixed `skillsmp/`):

| Tool | Description |
|------|-------------|
| `skillsmp_search` | Keyword search across all skills |
| `skillsmp_ai_search` | Natural-language semantic search |
| `skillsmp_get_skill_content` | Fetch a skill's `SKILL.md` from GitHub |
| `skillsmp_list_repo_skills` | List skills in a repository |
| `skillsmp_install_skill` | Install a skill to a local agent |

## Running the example

```bash
cd examples/skillsmp-agent

# Set your LLM API key
export ANTHROPIC_API_KEY=<your-key>

# (Option A) Install static skills
npx skills add anthropics/claude-code/frontend-design
npx skills add vercel-labs/agent-skills/find-skills

# (Option B) Start SkillsMP MCP server
export SKILLSMP_API_KEY=<your-key>
npx skillsmp-mcp-server --transport http --port 3010

# Deploy the agent
rune daemon start --foreground &
rune compose up -f rune-compose.yml

# Run it
rune run --agent skillsmp-agent "Help me design a responsive landing page."
rune run --agent skillsmp-agent "Find a skill for writing Rust documentation."
```

## File structure

```
skillsmp-agent/
├── Runefile                                              # Agent definition
└── skills/                                               # Locally installed skills
    └── anthropics/claude-code/frontend-design/
        └── SKILL.md                                      # Placeholder — install with npx skills add
```
