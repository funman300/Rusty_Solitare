#!/usr/bin/env bash
# Rebuild the solitaire_wasm crate and install the output into
# solitaire_server/web/pkg/ so the server can serve the replay viewer.
#
# Prerequisites:
#   cargo install wasm-pack
#   rustup target add wasm32-unknown-unknown
#
# Run from the repo root:
#   ./build_wasm.sh
#
# The generated files (solitaire_wasm.js + solitaire_wasm_bg.wasm) are
# committed to git so self-hosters who don't touch the WASM crate can
# skip this step.  Regenerate after any change to solitaire_wasm/ or
# solitaire_core/.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT_DIR="$REPO_ROOT/solitaire_server/web/pkg"

if ! command -v wasm-pack &> /dev/null; then
    echo "error: wasm-pack not found." >&2
    echo "  Install with: cargo install wasm-pack" >&2
    exit 1
fi

echo "Building solitaire_wasm (target: web)..."
wasm-pack build \
    --target web \
    --out-dir "$OUT_DIR" \
    --no-typescript \
    "$REPO_ROOT/solitaire_wasm"

# wasm-pack writes a package.json and .gitignore into the output dir.
# Remove them — we manage the output directory ourselves.
rm -f "$OUT_DIR/package.json" "$OUT_DIR/.gitignore"

echo "Done. Output:"
ls -lh "$OUT_DIR"
