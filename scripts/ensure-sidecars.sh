#!/usr/bin/env bash
# Make sure Tauri's externalBin sidecars exist for this host, and match their sources, before
# `tauri dev`.
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

# A binary is rebuilt when it is missing or its `.stamp` (the source hash written by its build
# script) no longer matches the sources, so `tauri dev` never runs a stale sidecar (TEST-009).
needs_build() {
    local bin="$1" kind="$2" path="$OUT/${1}-${TRIPLE}"
    if [[ ! -x "$path" ]]; then
        echo "→ missing $path"
        return 0
    fi
    if [[ "$(cat "$path.stamp" 2>/dev/null || true)" != "$(bash "$ROOT/scripts/sidecar-stamp.sh" "$kind")" ]]; then
        echo "→ $path is older than its sources"
        return 0
    fi
    return 1
}

if needs_build bluey-helper helper; then
    echo "→ building the host helper for $TRIPLE"
    bash "$ROOT/scripts/build-helper.sh" host
fi
if needs_build bluey-agent agent; then
    echo "→ building the host agent for $TRIPLE"
    bash "$ROOT/scripts/build-agent.sh" host
fi
