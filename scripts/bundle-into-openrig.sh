#!/usr/bin/env bash
# Validates every plugin under plugins/source/ and writes a single zip
# bundle into the sibling OpenRig repo's plugins/ directory.
#
# That zip is what the OpenRig installer ships and the app extracts on
# first launch into the OS plugins dir (~/Library/Application Support/
# OpenRig/plugins on macOS, %APPDATA%/OpenRig/plugins on Windows,
# ~/.local/share/openrig/plugins on Linux).
#
# Usage: ./scripts/bundle-into-openrig.sh [<openrig-repo-path>]
# Default openrig path: sibling directory ../OpenRig

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGINS_REPO="$(cd "$SCRIPT_DIR/.." && pwd)"
OPENRIG_REPO="${1:-$(cd "$PLUGINS_REPO/../OpenRig" && pwd)}"

if [ ! -d "$OPENRIG_REPO" ]; then
    echo "openrig repo not found at $OPENRIG_REPO" >&2
    echo "usage: $0 [<openrig-repo-path>]" >&2
    exit 1
fi

OUT_DIR="$OPENRIG_REPO/plugins"
OUT_ZIP="$OUT_DIR/openrig-plugins.zip"

mkdir -p "$OUT_DIR"

cd "$PLUGINS_REPO"
cargo run --quiet --bin pack_plugins -- --bundle "$OUT_ZIP"

echo
echo "wrote $OUT_ZIP ($(du -h "$OUT_ZIP" | cut -f1))"
