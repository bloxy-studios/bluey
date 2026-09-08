#!/usr/bin/env bash
# Production build pipeline (run on macOS with Xcode command line tools, Bun and Rust).
#
#   scripts/release.sh                 # universal-ready .app + .dmg for the host architecture
#   TARGET=aarch64-apple-darwin scripts/release.sh
#
# Steps: install deps → typecheck/lint/test → build Swift helper + Bun agent sidecars for the
# target → tauri build (frontend + Rust + bundle) → sign/notarize when credentials exist.
#
# Research sidecar variant: the default build is the **lite** bluey-agent (Gemini function
# calling over @google/genai — the default RESEARCH_BACKEND). Set RESEARCH_BACKEND=claude (or
# BLUEY_AGENT_VARIANT=full) to embed the Claude CLI for the Claude Agent SDK backend.
#
# Signing/notarization env (all optional — skipped when absent):
#   APPLE_SIGNING_IDENTITY   e.g. "Developer ID Application: Name (TEAMID)"
#   APPLE_ID, APPLE_PASSWORD (app-specific), APPLE_TEAM_ID   → notarization via tauri
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.bun/bin:$HOME/.cargo/bin:$PATH"

TARGET="${TARGET:-$(rustc --print host-tuple)}"
case "$TARGET" in
  aarch64-apple-darwin|x86_64-apple-darwin) ;;
  *) echo "✗ Bluey release builds target macOS only (got $TARGET)"; exit 1 ;;
esac

echo "▶ bun install"
bun install --frozen-lockfile || bun install

echo "▶ frontend checks"
bun run typecheck
bun run lint
bun run test

echo "▶ rust checks"
bash scripts/check-rust.sh

echo "▶ native helper ($TARGET)"
TARGET="$TARGET" bash scripts/build-helper.sh

AGENT_VARIANT="${BLUEY_AGENT_VARIANT:-}"
if [[ -z "$AGENT_VARIANT" ]]; then
  case "${RESEARCH_BACKEND:-gemini}" in
    claude|anthropic) AGENT_VARIANT=full ;;
    *) AGENT_VARIANT=lite ;;
  esac
fi
echo "▶ agent sidecar ($TARGET, $AGENT_VARIANT variant)"
BLUEY_AGENT_VARIANT="$AGENT_VARIANT" TARGET="$TARGET" bash scripts/build-agent.sh

for bin in bluey-helper bluey-agent; do
  if [[ ! -x "src-tauri/binaries/${bin}-${TARGET}" ]]; then
    echo "✗ missing sidecar binary src-tauri/binaries/${bin}-${TARGET}"; exit 1
  fi
done

echo "▶ tauri build"
if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  echo "  signing with: $APPLE_SIGNING_IDENTITY"
  export APPLE_SIGNING_IDENTITY
  if [[ -n "${APPLE_ID:-}" && -n "${APPLE_PASSWORD:-}" && -n "${APPLE_TEAM_ID:-}" ]]; then
    echo "  notarization credentials present — tauri will notarize"
    export APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID
  fi
else
  echo "  no APPLE_SIGNING_IDENTITY — producing an ad-hoc signed build (not distributable)"
fi
bun run tauri build --target "$TARGET"

echo "✓ artifacts:"
ls -1 "src-tauri/target/$TARGET/release/bundle/macos/" 2>/dev/null || true
ls -1 "src-tauri/target/$TARGET/release/bundle/dmg/" 2>/dev/null || true
