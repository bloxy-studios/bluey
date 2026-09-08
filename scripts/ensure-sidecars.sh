#!/usr/bin/env bash
# Make sure Tauri's externalBin sidecars exist for this host before `tauri dev`.
# Full dual-arch binaries are still produced by `bun run build:helpers` / release.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/src-tauri/binaries"

case "$(uname -m)" in
    arm64)  TRIPLE="aarch64-apple-darwin" ;;
    x86_64) TRIPLE="x86_64-apple-darwin" ;;
    *)
        echo "error: unsupported host arch $(uname -m)" >&2
        exit 1
        ;;
esac

missing=0
for bin in bluey-helper bluey-agent; do
    if [[ ! -x "$OUT/${bin}-${TRIPLE}" ]]; then
        echo "→ missing $OUT/${bin}-${TRIPLE}"
        missing=1
    fi
done

if [[ "$missing" -eq 0 ]]; then
    exit 0
fi

echo "→ building host sidecars for $TRIPLE (first run; later tauri dev skips this)"
bash "$ROOT/scripts/build-helper.sh" host
bash "$ROOT/scripts/build-agent.sh" host
