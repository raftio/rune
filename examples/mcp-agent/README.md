# MCP Agent Example

An agent that connects to external MCP servers (Filesystem + GitHub) and uses their tools alongside Rune's built-in tools.

**What this example shows:**
- `mcp_servers:` in the Runefile — declaring external MCP servers
- Tool name prefixing: `{server-name}/{tool-name}` (e.g. `filesystem/read_file`)
- Using the Rune gateway as an MCP server from Claude Desktop / Cursor

## Prerequisites

Start the MCP servers (requires Node.js):

```bash
# Filesystem MCP server — exposes read_file, write_file, list_directory
npx -y @modelcontextprotocol/server-filesystem /tmp/workspace &

# GitHub MCP server — exposes search_repositories, create_issue, etc.
export GITHUB_TOKEN=ghp_...
npx -y @modelcontextprotocol/server-github &
```

The Filesystem server listens on stdio by default. To expose it over HTTP (required by Rune's MCP client), wrap it with [mcp-proxy](https://github.com/modelcontextprotocol/mcp-proxy):

```bash
npx -y mcp-proxy --port 3001 -- npx @modelcontextprotocol/server-filesystem /tmp/workspace
npx -y mcp-proxy --port 3002 -- npx @modelcontextprotocol/server-github
```

## Run

```bash
cd examples/mcp-agent
export ANTHROPIC_API_KEY=<your-key>
export GITHUB_TOKEN=<your-github-token>

rune daemon start --foreground &
rune compose up -f rune-compose.yml

rune run --agent mcp-agent "List the files in /tmp/workspace"
rune run --agent mcp-agent "Search GitHub for repositories about Raft consensus"
```

## Using Rune as an MCP server

Once the daemon is running with any agent deployed, you can add Rune to Claude Desktop:

`~/Library/Application Support/Claude/claude_desktop_config.json`:
```json
{
  "mcpServers": {
    "rune": {
      "url": "http://localhost:8080/mcp",
      "type": "http"
    }
  }
}
```

Every deployed Rune agent appears as a tool in Claude. Call `mcp-agent` with a natural language prompt and it orchestrates the filesystem and GitHub tools for you.

## Tool name convention

MCP tools are prefixed with the `name` from `mcp_servers:`:

```
mcp_servers:
  - name: filesystem   →  filesystem/read_file, filesystem/write_file, ...
  - name: github       →  github/search_repositories, github/create_issue, ...
```

The same prefixed name goes in `toolset:` so the LLM knows it's available.

## Environment overrides

Override MCP server URLs at deploy time without changing the Runefile:

```bash
RUNE_MCP_FILESYSTEM_URL=http://prod-mcp:3001/mcp \
RUNE_MCP_GITHUB_URL=http://prod-mcp:3002/mcp \
  rune compose up -f rune-compose.yml
```
