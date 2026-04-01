#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")" && pwd)"
INSTALL_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "$INSTALL_DIR"

echo "Building ferro-wg and ferro-wg-daemon (release mode)..."
cargo build --release --manifest-path "$REPO_DIR/Cargo.toml" -p ferro-wg -p ferro-wg-daemon

cp "$REPO_DIR/target/release/ferro-wg" "$INSTALL_DIR/ferro-wg"
cp "$REPO_DIR/target/release/ferro-wg-daemon" "$INSTALL_DIR/ferro-wg-daemon"

echo "Installed ferro-wg to $INSTALL_DIR/ferro-wg"
echo "Installed ferro-wg-daemon to $INSTALL_DIR/ferro-wg-daemon"

if command -v ferro-wg >/dev/null 2>&1; then
    echo ""
    ferro-wg --version
    echo ""
    echo "Usage:"
    echo "  ferro-wg import <wg-quick.conf>   Import a WireGuard config"
    echo "  sudo ferro-wg-daemon -c <config>  Start the privileged daemon"
    echo "  ferro-wg up                       Bring up tunnel(s)"
    echo "  ferro-wg status                   Show connection status"
    echo "  ferro-wg tui                      Launch interactive TUI"
else
    echo ""
    echo "Make sure $INSTALL_DIR is in your PATH:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
fi
