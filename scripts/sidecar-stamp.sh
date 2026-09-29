#!/usr/bin/env bash
# Print the source hash of a sidecar: build-helper.sh / build-agent.sh write it next to each
# binary as `<binary>.stamp`, and ensure-sidecars.sh rebuilds a binary whose stamp no longer
# matches, so `tauri dev` never runs a helper or agent older than its sources (TEST-009).
# Usage: bash scripts/sidecar-stamp.sh helper|agent
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

case "${1:-}" in
    helper)
        set -- src-tauri/swift/BlueyHelper/Package.swift src-tauri/swift/BlueyHelper/Sources \
            src-tauri/swift/BlueyHelper/bluey-helper.entitlements scripts/build-helper.sh
        ;;
    agent)
        set -- sidecars/agent/package.json sidecars/agent/bun.lock sidecars/agent/src \
            scripts/build-agent.sh
        ;;
    *)
        echo "usage: $0 helper|agent" >&2
        exit 2
        ;;
esac

# Paths and contents both count: a renamed or deleted file changes the hash too.
find "$@" -type f ! -name .DS_Store -print0 \
    | LC_ALL=C sort -z \
    | xargs -0 shasum -a 256 \
    | shasum -a 256 \
    | cut -d ' ' -f 1
