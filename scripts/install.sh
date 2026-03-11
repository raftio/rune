#!/usr/bin/env bash
set -e

# Install rune CLI
# Usage: curl -fsSL <url>/install.sh | bash
#        or: ./install.sh

BIN_NAME="rune"
INSTALL_DIR="${RUNE_INSTALL:-$HOME/.local/bin}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

main() {
  if ! command -v cargo &>/dev/null; then
    echo "Error: cargo not found. Install Rust from https://rustup.rs" >&2
    exit 1
  fi

  echo "Building $BIN_NAME..."
  cargo build --release -p rune

  mkdir -p "$INSTALL_DIR"
  cp "../target/release/$BIN_NAME" "$INSTALL_DIR/$BIN_NAME"
  chmod +x "$INSTALL_DIR/$BIN_NAME"

  if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    echo ""
    echo "Installed to $INSTALL_DIR/$BIN_NAME"
    echo "Add to PATH: export PATH=\"\$PATH:$INSTALL_DIR\""
  else
    echo "Installed to $INSTALL_DIR/$BIN_NAME"
  fi
}

main "$@"
