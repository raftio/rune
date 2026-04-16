# rune-artifact

Library for packing a local agent directory into a **deterministic** gzip-compressed tar (`.tar.gz` or `.tgz`) with a versioned `manifest.json` (file SHA-256s), and for verifying or extracting that artifact for runtime use.

Longer background: [Open Agent Initiative white paper](../../docs/concepts/open-agent-initiative.md).

## Open Agent Initiative (OAI) vs OCI

**Open Agent Initiative (OAI)** is the working name for this **portable agent bundle** format in Rune: a tarball tree that `rune_spec::AgentPackage::load` can consume. It is **not** an [Open Container Initiative](https://opencontainers.org/) (**OCI**) **container** image. There is no container rootfs or image runtime implied—only a reproducible archive of `Runefile`, tools, skills, and metadata.

Packs produced by current tooling set `manifest.json` field `initiative` to `"open-agent"` so tools can recognize OAI-aligned artifacts. Older archives may omit this field; `verify` still accepts them.

## Why `.tar.gz`?

The format is intentionally plain **tar + gzip**: widely supported, easy to inspect (`tar -tzf`), scriptable in CI, and deterministic when entries are written in sorted order. It is a **local and transport** format first; remote registry integration is optional and separate.

## Layout

After extraction, the tree is rooted at `agent/`:

- `agent/manifest.json` — `format: rune-artifact-v1`, optional `initiative: open-agent`, `tag` (from `rune artifact build --tag`, default `latest`), agent name/version, RFC3339 `created_at`, and a sorted `files` list (paths relative to `agent/`, excluding `manifest.json`).
- `agent/Runefile`, `agent/tools/…`, `agent/skills/…`, optional `agent/workflow.yaml`.

## Loading with `AgentPackage`

The extracted `agent/` directory matches what `rune_spec::AgentPackage::load` expects: point it at the **`agent/` root** (the path returned by `extract_to_temp` / `extract_and_load_package`).

```rust
let (_tmp, agent_root) = rune_artifact::extract_to_temp(std::fs::File::open("out.tar.gz")?)?;
let pkg = rune_spec::AgentPackage::load(&agent_root)?;
```

Or use `extract_and_load_package` to get `(TempDir, AgentPackage)` in one step.

## Remote skills (`missing_skills`)

The packer only includes **on-disk** files under the agent directory. Skill references in `Runefile` that are not present under `skills/` are **not** resolved or embedded. After load, behavior matches a dev tree: those entries appear in `missing_skills` as today.

## CLI

From the workspace `rune` binary:

```bash
# Writes ~/.rune/artifacts/{agent-name}-{tag}.tar.gz (tag defaults to latest)
cargo run -p rune -- artifact build /path/to/agent

cargo run -p rune -- artifact build /path/to/agent --tag v1.0.0
cargo run -p rune -- artifact verify my-agent
cargo run -p rune -- artifact verify my-agent v1.0.0
```

Defaults:

- **Agent directory:** required **positional path** on `artifact build` (directory containing `Runefile`).
- **Output:** always **`~/.rune/artifacts/{agent-name}-{tag}.tar.gz`** (directory is created as needed; no `-o`).
- **`--tag`:** defaults to **`latest`** for both the filename and the manifest `tag` field.
- **`artifact verify NAME [TAG]`:** checks the same path pattern; **`TAG`** defaults to **`latest`** if omitted.

The build command prints **`artifact_sha256`**, **`tag=`**, and the output path. Use **`Runefile` `version`**, **`--tag`**, and **git** for version control; there is no remote registry in this flow.

## Artifact digest

The manifest does not include a self-referential hash of the full archive; the digest of the shipped bytes is the value printed after a successful `pack` / `artifact build`.
