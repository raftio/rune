# Code Assistant Agent

An agent that reads, writes, and patches files and runs shell commands — a minimal local coding assistant.

**What this example shows:**
- `rune@file-read` / `rune@file-write` / `rune@file-list` — filesystem access within the workspace
- `rune@apply-patch` — applying unified diffs for targeted edits
- `rune@shell-exec` — running tests, linters, and build commands

## Run

```bash
cd examples/code-assistant
export ANTHROPIC_API_KEY=<your-key>

# Point the workspace at your project
export RUNE_WORKSPACE_DIR=/path/to/your/project

rune daemon start --foreground &
rune compose up -f rune-compose.yml
```

Example prompts:

```bash
rune run --agent code-assistant "List the files in src/ and tell me what the project does."

rune run --agent code-assistant \
  "Add a docstring to every public function in src/utils.py and run the tests."

rune run --agent code-assistant \
  "The test suite is failing. Read the failing test, find the bug, fix it, and run tests again."
```

## Security note

`rune@shell-exec` runs commands in the agent's process environment. In production, run the daemon inside a container or with a restricted user. Set `RUNE_WORKSPACE_DIR` to limit filesystem access to your project root.
