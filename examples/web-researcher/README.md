# Web Researcher Agent

An agent that searches the web, reads pages, and builds a knowledge graph from what it learns.

**What this example shows:**
- `rune@web-search` — DuckDuckGo search, no API key needed
- `rune@web-fetch` — reading full pages (SSRF-protected)
- `rune@knowledge-add-entity` / `rune@knowledge-add-relation` — building a persistent knowledge graph
- `rune@knowledge-query` — querying what has already been learned

## Run

```bash
cd examples/web-researcher
export ANTHROPIC_API_KEY=<your-key>

rune daemon start --foreground &
rune compose up -f rune-compose.yml
```

Ask a research question:

```bash
rune run --agent web-researcher "What is Raft consensus and who created it?"
# Searches, fetches, answers with citations, adds entities to knowledge graph

rune run --agent web-researcher "What do you know about Raft?"
# Queries knowledge graph — no web fetch needed
```

## How it works

1. Agent searches with `rune@web-search` (up to 20 results).
2. Fetches top pages with `rune@web-fetch` (HTML auto-truncated).
3. Writes key entities (`person`, `concept`, `project`) and relations to the knowledge graph.
4. On subsequent queries it first checks `rune@knowledge-query` before going to the web.

The knowledge graph is stored in SQLite and persists across restarts.
