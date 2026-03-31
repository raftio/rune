# Open Agent Initiative (OAI)

**White paper (working draft)** — portable agent artifacts in Rune

## Abstract

The **Open Agent Initiative (OAI)** is a **working name** for Rune’s **portable agent bundle** format: a deterministic, verifiable archive that contains everything a runtime needs to load an agent from disk (`Runefile`, tools, skills, optional workflow metadata). OAI is **not** the [Open Container Initiative](https://opencontainers.org/) (OCI): it does not define container images, root filesystems, or image runtimes. It defines an **agent** package suitable for local builds, CI, and—when needed later—remote storage without conflating that payload with Docker/OCI semantics.

## Motivation

Agents need a **clear boundary** between “source tree in development” and “shippable unit” that can be:

- **Reproduced** from the same inputs (deterministic layout and hashing).
- **Verified** after download or copy (per-file SHA-256 in a manifest).
- **Named and versioned** without implying a container registry (`Runefile` version, optional local `tag`, output filename, git).

OAI provides that boundary in Rune today as **`rune-artifact-v1`**: gzip-compressed tar with an `agent/manifest.json` and an `agent/` tree compatible with `AgentPackage::load`.

## Terminology

| Term | Meaning |
|------|--------|
| **OAI** | Open Agent Initiative — Rune’s label for this portable bundle format (documentation and `manifest.json` field `initiative: "open-agent"` when produced by current tooling). |
| **OCI** | Open Container Initiative — industry standards for **container** images and runtimes. OAI artifacts are **not** OCI images. |
| **`rune-artifact-v1`** | On-disk format version string in `manifest.json` (`format`). |
| **Artifact digest** | SHA-256 of the **entire** `.tar.gz` bytes (printed by `rune artifact build`; not self-referentially embedded in `manifest.json` in v1). |

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

- **`Runefile`** `name` and `version` as the canonical semantic identity.
- **Agent path** only via **`AGENT_DIR`** in `~/.rune/config.toml` (`rune artifact build` does not accept a directory path on the CLI).
- **Output** is always `~/.rune/artifacts/{agent-name}-{tag}.tar.gz`; **`--tag`** defaults to **`latest`** in the filename and manifest.
- **`rune artifact build --tag`** `LABEL` sets the tag when you need a label other than `latest`.
- **Git** for source history; `--tag` can mirror git tags when useful.

## Security and trust

- **Integrity:** `verify` validates file hashes against the manifest.
- **Authenticity** (signing, provenance) is **out of scope** for v1; may be layered later (e.g. sidecar signatures or registry attestations).

## Relationship to Rune deployment

The control plane may store **`image_ref`** / **`image_digest`** for **runtime** images (e.g. WASM or Docker backends). Those fields refer to **how replicas run**, not necessarily to the OAI tarball. Wiring **artifact digest** into registration as a first-class field is a possible follow-up when remote artifact storage is required.

## Roadmap (non-normative)

- **Current:** Local pack, verify, extract; CLI `rune artifact build` / `verify`.
- **Deferred:** Remote registry or object store, optional OCI Distribution **as transport only** for the same bytes (still not claiming an OAI bundle is a “container image”), control-plane fields for artifact digest.

## References

- Implementation: `crates/rune-artifact` (crate README and API).
- Agent loading: `rune_spec::AgentPackage::load`.

---

*This document is a working draft and may evolve with the format; `format: rune-artifact-v1` and migration policy will be versioned with any breaking change.*
