#!/usr/bin/env bash
# Run the BlueyHelperCore unit tests (pure logic: dHash, VAD, chunker, envelope
# coding, OCR ordering, AX role filter — no device/TCC access required).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PKG_DIR="$REPO_ROOT/src-tauri/swift/BlueyHelper"

if ! command -v swift >/dev/null 2>&1; then
    echo "error: \`swift\` not found — install Xcode or the Command Line Tools (macOS only)." >&2
    exit 1
fi

exec swift test --package-path "$PKG_DIR" "$@"
