# Model Context Protocol (MCP)

Rune supports the [Model Context Protocol](https://modelcontextprotocol.io) in both directions:

- **MCP Client** — agents can connect to external MCP servers and use their tools
- **MCP Server** — Rune exposes all deployed agents as MCP tools at `POST /mcp`

## MCP Server — using Rune from Claude Desktop / Cursor

Any MCP-compatible client can connect to the Rune gateway and call agents as tools.

**Endpoint:** `POST http://localhost:8080/mcp`

**Supported methods:**

| Method | Description |
|--------|-------------|
| `initialize` | Handshake — returns `protocolVersion`, `capabilities`, `serverInfo` |
| `tools/list` | Returns all active deployed agents as MCP tools |
| `tools/call` | Invokes an agent by name with `{ input, session_id? }` |
| `ping` | Health check |

### Configure in Claude Desktop

Add to `~/Library/Application Support/Claude/claude_desktop_config.json`:

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

After restarting Claude Desktop, all your deployed Rune agents appear as tools.

### Configure in Cursor / VS Code

Add to your MCP settings:

```json
{
  "mcp": {
    "servers": {
      "rune": {
        "url": "http://localhost:8080/mcp",
        "type": "http"
      }
    }
  }
}
```

## MCP Client — agents consuming external MCP servers

Agents can connect to external MCP servers and use their tools. Declare them in the `Runefile` under `mcp_servers:`:

```yaml
name: my-agent
version: 0.1.0
instructions: |
  Use the filesystem and GitHub tools to help the user.
default_model: default
toolset:
  - rune@memory-store        # built-in
  - filesystem/read_file     # from MCP server "filesystem"
  - filesystem/write_file
  - github/create_issue      # from MCP server "github"

mcp_servers:
  - name: filesystem
    url: http://localhost:3001/mcp

  - name: github
    url: http://localhost:3002/mcp
    headers:
      Authorization: "Bearer ${GITHUB_TOKEN}"

runtime:
  streaming_enabled: true
  resource_profile: small

models:
  providers:
    - anthropic
  model_mapping:
    default: claude-sonnet-4-6
```

### How it works

1. At agent load time, Rune calls `initialize` + `tools/list` on each MCP server
2. Discovered tools are injected into the agent's toolset with the prefix `{server-name}/{tool-name}`
3. When the LLM calls a tool like `filesystem/read_file`, the `ToolDispatcher` routes it to the MCP server via `tools/call`
4. Results are returned to the LLM as tool outputs

### Tool naming

MCP tools are prefixed with the server `name` from `mcp_servers:`:

| MCP server `name` | MCP tool name | Tool name in Rune |
|-------------------|---------------|-------------------|
| `filesystem` | `read_file` | `filesystem/read_file` |
| `github` | `create_issue` | `github/create_issue` |

Add the prefixed names to `toolset:` so the LLM knows they're available.

### Server URL from environment

The MCP server URL can be overridden at runtime via environment variables:

```
RUNE_MCP_{SERVER_NAME}_URL=http://host:port/mcp
```

Where `{SERVER_NAME}` is the uppercased `name` with hyphens replaced by underscores:

| `name` in Runefile | Env var |
|--------------------|---------|
| `filesystem` | `RUNE_MCP_FILESYSTEM_URL` |
| `my-server` | `RUNE_MCP_MY_SERVER_URL` |

### Tool descriptor (manual)

If you prefer not to use `mcp_servers:` auto-discovery, you can declare MCP tools manually in `tools/`:

```yaml
# tools/filesystem_read.yaml
name: filesystem/read_file
version: 0.1.0
runtime: mcp
mcp_server: filesystem
timeout_ms: 5000
```

## Popular MCP Servers

| Server | Tools | Run |
|--------|-------|-----|
| [Filesystem](https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem) | `read_file`, `write_file`, `list_directory` | `npx @modelcontextprotocol/server-filesystem /path` |
| [GitHub](https://github.com/modelcontextprotocol/servers/tree/main/src/github) | `create_issue`, `get_pr`, `search_repos` | `npx @modelcontextprotocol/server-github` |
| [Fetch](https://github.com/modelcontextprotocol/servers/tree/main/src/fetch) | `fetch` | `npx @modelcontextprotocol/server-fetch` |
| [Brave Search](https://github.com/modelcontextprotocol/servers/tree/main/src/brave-search) | `brave_web_search` | `npx @modelcontextprotocol/server-brave-search` |
| [Postgres](https://github.com/modelcontextprotocol/servers/tree/main/src/postgres) | `query` | `npx @modelcontextprotocol/server-postgres postgresql://...` |
| [Puppeteer](https://github.com/modelcontextprotocol/servers/tree/main/src/puppeteer) | `navigate`, `screenshot`, `click` | `npx @modelcontextprotocol/server-puppeteer` |

Start any MCP server and add it to `mcp_servers:` in your Runefile.
