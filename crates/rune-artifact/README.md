# rune-artifact

Library for **materializing** a local agent directory into an unpacked OAI bundle (`agent/manifest.json` + packable files), **exporting** that tree to a deterministic gzip-compressed tar (`.tar.gz`), and for **verifying** or **extracting** bundles for runtime use.

## Open Agent Initiative (OAI)

**Open Agent Initiative (OAI)** is the working name for this **portable agent bundle** format in Rune: a tree that `rune_spec::AgentPackage::load` can consume from the `agent/` root — reproducible `Runefile` plus metadata.

Packs produced by current tooling set `manifest.json` field `initiative` to `"open-agent"` so tools can recognize OAI-aligned artifacts. Older archives may omit this field; `verify` still accepts them.

## Canonical layout vs `.tar.gz`

The **canonical on-disk store** under `~/.rune/artifacts/` is a **directory** `{agent-name}-{tag}/` containing `agent/` (same layout as after tar extraction). **`.tar.gz`** is the **transport** form: use `export_bundle_to_tar_gz_file` from this crate or `rune artifact export`.

**Tar + gzip** stays the interchange format: widely supported, easy to inspect (`tar -tzf`), scriptable in CI, and deterministic when entries are written in sorted order.

## Layout

The tree is rooted at `agent/`:

- `agent/manifest.json` — `format: rune-artifact-v1`, optional `initiative: open-agent`, `tag` (from `rune artifact build --tag`, default `latest`), agent name, model, RFC3339 `created_at`, and a sorted `files` list (paths relative to `agent/`, excluding `manifest.json`).
- `agent/Runefile`.

## Loading with `AgentPackage`

Point `AgentPackage::load` at the **`agent/` root** (e.g. `{bundle}/agent` after materialize, or the path returned by `extract_to_temp` / `extract_and_load_package`).

```rust
// From a materialized bundle directory:
let agent_root = std::path::Path::new("/path/to/my-agent-latest/agent");
let pkg = rune_spec::AgentPackage::load(agent_root)?;

// From a .tar.gz file:
let (_tmp, agent_root) = rune_artifact::extract_to_temp(std::fs::File::open("out.tar.gz")?)?;
let pkg = rune_spec::AgentPackage::load(&agent_root)?;
```

Or use `extract_and_load_package` to get `(TempDir, AgentPackage)` in one step.

## CLI

From the workspace `rune` binary:

```bash
# Writes ~/.rune/artifacts/{agent-name}-{tag}/agent/... (tag defaults to latest)
cargo run -p rune -- artifact build /path/to/agent

cargo run -p rune -- artifact build /path/to/agent --tag v1.0.0

# Creates ~/.rune/artifacts/{name}-{tag}.tar.gz from that directory
cargo run -p rune -- artifact export my-agent
cargo run -p rune -- artifact export my-agent v1.0.0
cargo run -p rune -- artifact export my-agent --output /tmp/out.tar.gz

cargo run -p rune -- artifact inspect my-agent
cargo run -p rune -- artifact inspect my-agent v1.0.0
cargo run -p rune -- artifact ls
```

Defaults:

- **Agent directory:** required **positional path** on `artifact build` (directory containing `Runefile`).
- **Build output:** **`~/.rune/artifacts/{agent-name}-{tag}/`** with `agent/manifest.json` and packable files (replaces `agent/` if it already exists).
- **`--tag`:** defaults to **`latest`** for the directory name and the manifest `tag` field.
- **`artifact export`:** reads the materialized bundle directory; **`--output`** defaults to **`~/.rune/artifacts/{name}-{tag}.tar.gz`**. Prints **`artifact_sha256`** (digest of the `.tar.gz` bytes).
- **`artifact inspect`:** prefers the **bundle directory** if present; otherwise falls back to **`~/.rune/artifacts/{NAME}-{TAG}.tar.gz`** for legacy archives.

Use **`Runefile` `version`**, **`--tag`**, and **git** for version control; there is no remote registry in this flow.

## Artifact digest

The manifest does not include a self-referential hash of the full archive. The digest of the shipped **`.tar.gz`** bytes is printed after a successful **`export`** (or after `pack_agent_dir` / `pack_agent_dir_to_file` in the library).
