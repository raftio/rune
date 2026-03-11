# Installation

Rune is written in Rust. You need Cargo (via [rustup](https://rustup.rs)) to build the CLI.

## Prerequisites

- **Rust** — Install from [rustup.rs](https://rustup.rs)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Build from Source

### Option 1: Install Script

From the repository root:

```bash
./scripts/install.sh
```

This script:

1. Runs `cargo build --release -p rune`
2. Copies the binary to `$RUNE_INSTALL` (or `$HOME/.local/bin` by default)
3. Prompts you to add that directory to `PATH` if needed

### Option 2: Manual Build

```bash
git clone https://github.com/raftio/rune.git
cd rune
cargo build --release -p rune
```

The binary is at `target/release/rune`. Optionally, copy it to a directory in your `PATH`:

```bash
cp target/release/rune ~/.local/bin/
export PATH="$PATH:$HOME/.local/bin"
```

## Verify Installation

```bash
rune --version
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `RUNE_INSTALL` | `$HOME/.local/bin` | Directory where the install script copies the binary |
| `DATABASE_URL` | `sqlite:~/.rune/rune.db` | SQLite database path for standalone mode |
