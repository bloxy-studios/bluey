#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────────
# Build the bluey-agent research sidecar into single-file Bun executables and
# drop them where Tauri v2 expects externalBin sidecars:
#   src-tauri/binaries/bluey-agent-aarch64-apple-darwin
#   src-tauri/binaries/bluey-agent-x86_64-apple-darwin
#
# Variants (BLUEY_AGENT_VARIANT):
#   lite (default) — Gemini backend only needs `@google/genai` (pure JS): the
#                    binary is a few MB and embeds no Claude CLI. The Claude
#                    backend still works when BLUEY_CLAUDE_CLI points at a CLI.
#   full           — embeds the platform Claude Code CLI (~250 MB per arch) so
#                    RESEARCH_BACKEND=claude works with no external binary.
#   The default flips to `full` when RESEARCH_BACKEND=claude (or `anthropic`,
#   the same spellings scripts/release.sh accepts) is exported.
#
# Notes on the Claude CLI binary (full variant only):
#   The Claude Agent SDK ships its CLI as a NATIVE binary inside per-platform
#   optional dependencies (@anthropic-ai/claude-agent-sdk-darwin-{arm64,x64}).
#   Each compiled target embeds its own platform binary via
#   `import ... with { type: "file" }` (see src/entry-darwin-*.ts), so BOTH
#   darwin packages must exist in node_modules regardless of the host arch.
#   `bun install --os darwin --cpu '*'` overrides bun's host filtering for
#   optional dependencies (flags verified against `bun install --help`,
#   bun 1.4.x: "--cpu  Override CPU architecture for optional dependencies",
#   "--os  Override operating system for optional dependencies").
#   On an older bun without these flags, install the packages explicitly:
#     bun add --optional @anthropic-ai/claude-agent-sdk-darwin-arm64 \
#                        @anthropic-ai/claude-agent-sdk-darwin-x64
#
# Codesigning:
#   Uses $BLUEY_CODESIGN_IDENTITY when set, otherwise falls back to ad-hoc
#   signing ("-"), which is enough for local development on Apple Silicon.
# ──────────────────────────────────────────────────────────────────────────────
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AGENT_DIR="$ROOT/sidecars/agent"
OUT_DIR="$ROOT/src-tauri/binaries"

if ! command -v bun >/dev/null 2>&1; then
  echo "error: bun is required to build the agent sidecar but was not found on PATH." >&2
  echo "       Install it from https://bun.sh (curl -fsSL https://bun.sh/install | bash)" >&2
  echo "       and re-run: bun run build:agent" >&2
  exit 1
fi

VARIANT="${BLUEY_AGENT_VARIANT:-}"
if [ -z "$VARIANT" ]; then
  # Same spelling set as scripts/release.sh and the sidecar's parseBackend().
  case "${RESEARCH_BACKEND:-gemini}" in
    claude|anthropic) VARIANT=full ;;
    *) VARIANT=lite ;;
  esac
fi
case "$VARIANT" in
  lite|full) ;;
  *)
    echo "error: BLUEY_AGENT_VARIANT must be 'lite' or 'full' (got '$VARIANT')" >&2
    exit 1
    ;;
esac
echo "→ agent variant: $VARIANT"

cd "$AGENT_DIR"

if [ "$VARIANT" = "full" ]; then
  echo "→ installing sidecar dependencies (including both darwin CLI binaries)"
  if ! bun install --os darwin --cpu '*'; then
    echo "error: bun install failed. If your bun predates --os/--cpu, run:" >&2
    echo "       bun add --optional @anthropic-ai/claude-agent-sdk-darwin-arm64 @anthropic-ai/claude-agent-sdk-darwin-x64" >&2
    exit 1
  fi
  # The embedded import fails at build time when a platform package is missing —
  # check up front for a clearer error.
  for pkg in claude-agent-sdk-darwin-arm64 claude-agent-sdk-darwin-x64; do
    if [ ! -f "node_modules/@anthropic-ai/$pkg/claude" ] && [ ! -f "$ROOT/node_modules/@anthropic-ai/$pkg/claude" ]; then
      echo "error: @anthropic-ai/$pkg is not installed — the compiled sidecar cannot embed the Claude CLI." >&2
      echo "       Run: bun add --optional @anthropic-ai/$pkg" >&2
      exit 1
    fi
  done
  ENTRY_SUFFIX=""
else
  echo "→ installing sidecar dependencies (lite: without the optional Claude CLI packages)"
  # The lite entry embeds no CLI and the Agent SDK resolves its platform package
  # only at runtime (never at bundle time), so the ~250 MB optional
  # @anthropic-ai/claude-agent-sdk-<platform> downloads are dead weight here.
  # --omit=optional leaves bun.lock untouched; a plain `bun install` (dev) or the
  # full build above re-adds the packages.
  bun install --omit=optional
  ENTRY_SUFFIX="-lite"
fi

mkdir -p "$OUT_DIR"

sign() {
  local file="$1"
  if ! command -v codesign >/dev/null 2>&1; then
    echo "  (codesign not available on this host — skipping signature for $file)"
    return 0
  fi
  if [ -n "${BLUEY_CODESIGN_IDENTITY:-}" ]; then
    echo "  codesigning with identity $BLUEY_CODESIGN_IDENTITY"
    codesign --force --options runtime --sign "$BLUEY_CODESIGN_IDENTITY" "$file"
  else
    echo "  codesigning ad-hoc (set BLUEY_CODESIGN_IDENTITY for a real identity)"
    codesign --force --sign - "$file"
  fi
}

build_target() {
  local entry="$1" target="$2" outfile="$3"
  echo "→ bun build --compile --target=$target ($entry)"
  bun build "$entry" --compile --target="$target" --outfile "$outfile"
  chmod +x "$outfile"
  sign "$outfile"
  echo "  built $(du -h "$outfile" | cut -f1 | tr -d ' ') → $outfile"
}

if [[ "${1:-}" == "host" ]]; then
    case "$(uname -m)" in
        arm64)
            build_target "./src/entry-darwin-arm64$ENTRY_SUFFIX.ts" bun-darwin-arm64 "$OUT_DIR/bluey-agent-aarch64-apple-darwin"
            ;;
        x86_64)
            build_target "./src/entry-darwin-x64$ENTRY_SUFFIX.ts" bun-darwin-x64 "$OUT_DIR/bluey-agent-x86_64-apple-darwin"
            ;;
        *)
            echo "error: unsupported host arch $(uname -m)" >&2
            exit 1
            ;;
    esac
else
    build_target "./src/entry-darwin-arm64$ENTRY_SUFFIX.ts" bun-darwin-arm64 "$OUT_DIR/bluey-agent-aarch64-apple-darwin"
    build_target "./src/entry-darwin-x64$ENTRY_SUFFIX.ts" bun-darwin-x64 "$OUT_DIR/bluey-agent-x86_64-apple-darwin"
fi

echo "✓ bluey-agent sidecar binaries built ($VARIANT)"
