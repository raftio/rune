# Open Agent Initiative (OAI)

**White paper (working draft)** — portable agent artifacts in Rune

## Abstract

The **Open Agent Initiative (OAI)** is a **working name** for Rune’s **portable agent bundle** format: a deterministic, verifiable archive that contains everything a runtime needs to load an agent from disk (`Runefile`, tools, skills, optional workflow metadata). OAI is **not** the [Open Container Initiative](https://opencontainers.org/) (OCI): it does not define container images, root filesystems, or image runtimes. It defines an **agent** package suitable for local builds, CI, and—when needed later—remote storage without conflating that payload with Docker/OCI semantics.

## Motivation

Agents need a **clear boundary** between “source tree in development” and “shippable unit” that can be:

- **Reproduced** from the same inputs (deterministic layout and hashing).
- **Verified** after copy or build (per-file SHA-256 in a manifest).
- **Addressed** by **name** and **tag** (like a simple coordinate) without a container registry—aligned with how the CLI stores and checks artifacts under `~/.rune/artifacts/`.

OAI provides that boundary in Rune today as **`rune-artifact-v1`**: an `agent/manifest.json` plus an `agent/` tree compatible with `AgentPackage::load`, stored canonically as a directory; a gzip-compressed tar (`.tar.gz`) is the portable export of the same tree.

## Terminology

| Term | Meaning |
|------|--------|
| **OAI** | Open Agent Initiative — Rune’s label for this portable bundle format (documentation and `manifest.json` field `initiative: "open-agent"` when produced by current tooling). |
| **OCI** | Open Container Initiative — industry standards for **container** images and runtimes. OAI artifacts are **not** OCI images. |
| **`rune-artifact-v1`** | On-disk format version string in `manifest.json` (`format`). |
| **Artifact digest** | SHA-256 of the **entire** `.tar.gz` bytes (printed by `rune artifact export` or library pack; not self-referentially embedded in `manifest.json` in v1). |
| **Artifact store path** | Default **bundle directory** **`~/.rune/artifacts/{name}-{tag}/`** (materialized `agent/` tree). Export writes **`~/.rune/artifacts/{name}-{tag}.tar.gz`**. `name` comes from the Runefile; `tag` is a CLI label (default **`latest`**). |

## CLI conventions (current)

The **`rune`** binary takes a **source path** for pack and **name/tag** coordinates for inspect:

| Command | Role |
|--------|------|
| **`rune artifact build`** `AGENT_DIR` `[--tag TAG]` | Materializes the agent at **`AGENT_DIR`** (must contain `Runefile`) into **`~/.rune/artifacts/{agent-name}-{tag}/agent/`**. **`--tag`** defaults to **`latest`** (directory name + manifest `tag`). |
| **`rune artifact export`** `NAME` `[TAG]` `[--output PATH]` | Writes **`~/.rune/artifacts/{NAME}-{TAG}.tar.gz`** from the materialized bundle directory (optional **`--output`**). **`TAG`** defaults to **`latest`**. |
| **`rune artifact inspect`** `NAME` `[TAG]` | Verifies the **bundle directory** if present; otherwise the legacy archive **`~/.rune/artifacts/{NAME}-{TAG}.tar.gz`**. Manifest fields and per-file integrity. **`TAG`** defaults to **`latest`**. |

The library crate **`rune-artifact`** exposes pack/verify/extract APIs for arbitrary paths for embedders and tests.

## Design principles

1. **Agent-first** — Layout matches what the Rune spec loader expects under `agent/` after extraction.
2. **Plain transport** — Tar + gzip for universal tooling and inspection (`tar -tzf`).
3. **Integrity** — Manifest lists every packed file (except `manifest.json` itself) with SHA-256; `verify` checks hashes.
4. **Determinism** — Files are walked and archived in sorted path order so identical trees yield comparable archives (subject to gzip timestamps/metadata as implemented).
5. **Explicit non-container** — No claim of compatibility with `docker run` or OCI image semantics for the bundle itself.

## Format overview (v1)

After extraction, the archive root contains:

- **`agent/manifest.json`** — JSON with at least: `format`, `agent_name`, `agent_version`, `created_at`, `files[]` with `{ path, sha256 }`. Optional: `initiative` (`open-agent`), `tag` (local label).
- **`agent/Runefile`**, **`agent/tools/...`**, **`agent/skills/...`**, optional **`agent/workflow.yaml`**.

**Remote skills:** OAI packs **on-disk content only**. Skill references in `Runefile` that are not present under `skills/` are not embedded; after load, `missing_skills` behaves as in a dev tree.

## Version control (local)

Recommended practice without a registry:

- **`Runefile`** `name` and `version` as the canonical semantic identity of the agent **spec**.
- **Source tree** passed explicitly as **`rune artifact build AGENT_DIR`** (path to the directory containing `Runefile`).
- **Artifact identity** via **`{name}`** (from Runefile at build time) and **`tag`** (`--tag` on build, default **`latest`** in directory name and manifest); **`rune artifact inspect`** / **`export`** use the same pair under **`~/.rune/artifacts/`**.
- **Git** for history; **`--tag`** can mirror git tags when useful.

## Security and trust

- **Integrity:** `verify` validates file hashes against the manifest.
- **Authenticity** (signing, provenance) is **out of scope** for v1; may be layered later (e.g. sidecar signatures or registry attestations).

## Relationship to Rune deployment

The control plane may store **`image_ref`** / **`image_digest`** for **runtime** images (e.g. WASM or Docker backends). Those fields refer to **how replicas run**, not necessarily to the OAI tarball. Wiring **artifact digest** into registration as a first-class field is a possible follow-up when remote artifact storage is required.

## Roadmap (non-normative)

- **Current:** Local materialize, export, and inspect via `rune artifact build` / `export` / `inspect`; library `extract` / `AgentPackage::load` for runtime; **no** remote registry in this iteration.
- **Deferred:** Remote registry or object store, optional OCI Distribution **as transport only** for the same bytes (still not claiming an OAI bundle is a “container image”), control-plane fields for artifact digest.

## References

- Implementation: `crates/rune-artifact` (crate README and API).
- Agent loading: `rune_spec::AgentPackage::load`.

---

*This document is a working draft and may evolve with the format; `format: rune-artifact-v1` and migration policy will be versioned with any breaking change.*
