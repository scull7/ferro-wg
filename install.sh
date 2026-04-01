#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")" && pwd)"
BINARY_NAME="ferro-wg"

echo "Building ferro-wg (release mode)..."
cargo build --release --manifest-path "$REPO_DIR/Cargo.toml" -p ferro-wg

INSTALL_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "$INSTALL_DIR"

cp "$REPO_DIR/target/release/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
echo "Installed $BINARY_NAME to $INSTALL_DIR/$BINARY_NAME"

if command -v "$BINARY_NAME" >/dev/null 2>&1; then
    echo ""
    "$BINARY_NAME" --version
    echo "Run 'ferro-wg --help' to get started."
else
    echo ""
    echo "Make sure $INSTALL_DIR is in your PATH:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
fi
