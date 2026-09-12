#!/usr/bin/env bash
# macOS-only build pipeline; nothing here creates a tag or GitHub Release.
# Default: developer build (.app + .dmg), NEVER eligible for publication.
# TARGET=... RESEARCH_BACKEND=gemini|claude bash scripts/release.sh
# PUBLISH_RELEASE=true is reserved for the gated Actions publishing workflow.
# Setup, credentials and native acceptance checks: docs/RELEASING.md.
set +x  # Never trace signing/notarization or optional OAuth inputs.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.bun/bin:$HOME/.cargo/bin:$PATH"

fail() { printf 'Release build error: %s\n' "$1" >&2; exit 1; }
PUBLISH_RELEASE="${PUBLISH_RELEASE:-false}"
case "$PUBLISH_RELEASE" in true|false) ;; *) fail 'PUBLISH_RELEASE must be true or false' ;; esac

# Every build signs its updater bundle (bundle.createUpdaterArtifacts); Tauri refuses to build without the key.
[[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" || -n "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}" ]] \
  || fail 'TAURI_SIGNING_PRIVATE_KEY (or TAURI_SIGNING_PRIVATE_KEY_PATH) is required to sign the updater bundle (docs/UPDATES.md › Signing)'

# Nightly builds carry a version above the sources (docs/UPDATES.md); publication builds never override.
BUILD_VERSION="${BLUEY_BUILD_VERSION:-}"
TAURI_CONFIG_ARGS=()
if [[ -n "$BUILD_VERSION" ]]; then
  [[ "$PUBLISH_RELEASE" == false ]] || fail 'BLUEY_BUILD_VERSION is only allowed for developer/nightly builds'
  python3 scripts/release/nightly.py check-version "$BUILD_VERSION" > /dev/null
  TAURI_CONFIG_ARGS=(--config "{\"version\":\"$BUILD_VERSION\"}")
fi

# Credentials and immutable version/tag checks precede installs, toolchains and builds.
if [[ "$PUBLISH_RELEASE" == true ]]; then
  python3 scripts/release/release_credentials.py check
  [[ "${GITHUB_ACTIONS:-}" == true && "${GITHUB_REPOSITORY:-}" == bloxy-studios/bluey ]] || fail 'publication eligibility is Actions-only in the canonical repository'
  : "${RELEASE_TAG:?existing release tag required}"
  : "${RELEASE_COMMIT:?preflight commit required}"
  : "${GITHUB_RUN_ID:?run ID required}"
  : "${GITHUB_RUN_ATTEMPT:?run attempt required}"
  : "${RUNNER_TEMP:?runner temporary directory required}"
  python3 scripts/release/release_metadata.py version --root "$ROOT" --tag "$RELEASE_TAG" --check-tag --commit "$RELEASE_COMMIT"
  # Do not allow alternate Tauri credential discovery or implicit certificate import.
  unset APPLE_CERTIFICATE APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH
else
  python3 scripts/release/release_metadata.py version --root "$ROOT"
fi

[[ "$(uname -s)" == Darwin ]] || fail 'building and native trust verification require macOS (no Linux stub)'
TARGET="${TARGET:-$(rustc --print host-tuple)}"
case "$TARGET" in
  aarch64-apple-darwin) DOWNLOAD_TARGET=mac-arm64 ;;
  x86_64-apple-darwin) DOWNLOAD_TARGET=mac-x64 ;;
  *) fail 'only aarch64-apple-darwin and x86_64-apple-darwin are supported' ;;
esac

case "${RESEARCH_BACKEND:-gemini}" in
  gemini) DEFAULT_VARIANT=lite ;;
  claude|anthropic) DEFAULT_VARIANT=full ;;
  *) fail 'RESEARCH_BACKEND must be gemini or claude' ;;
esac
AGENT_VARIANT="${BLUEY_AGENT_VARIANT:-$DEFAULT_VARIANT}"
case "$AGENT_VARIANT" in lite|full) ;; *) fail 'BLUEY_AGENT_VARIANT must be lite or full' ;; esac

# Explicit target directory is the audited Tauri output root, regardless of a local override.
export CARGO_TARGET_DIR="$ROOT/src-tauri/target"
BUNDLE_DIR="$CARGO_TARGET_DIR/$TARGET/release/bundle"
VERIFIED_DIR="${RUNNER_TEMP:-$ROOT}/bluey-verified/$DOWNLOAD_TARGET"
if [[ "$PUBLISH_RELEASE" == true ]]; then
  [[ ! -e "$VERIFIED_DIR" && ! -L "$VERIFIED_DIR" ]] || fail 'refusing a stale verified-artifact directory; use a fresh runner'
fi
# Remove old bundles, never build outputs for other targets. A failed build cannot reuse a DMG.
[[ ! -L "$CARGO_TARGET_DIR" && ! -L "$CARGO_TARGET_DIR/$TARGET" && ! -L "$CARGO_TARGET_DIR/$TARGET/release" ]] || fail 'symlinked target output is not supported'
rm -rf "$BUNDLE_DIR"

[[ "$(bun --version)" == 1.4.2 ]] || fail 'release builds require Bun 1.4.2'
export BLUEY_RELEASE_BUN
BLUEY_RELEASE_BUN="$(command -v bun)"
SHIM="$(mktemp -d "${TMPDIR:-/tmp}/bluey-release-bun.XXXXXX")"
trap 'rm -rf "$SHIM"' EXIT
cp scripts/release/bun-frozen.sh "$SHIM/bun"
chmod +x "$SHIM/bun"
export PATH="$SHIM:$PATH"

printf '%s\n' 'Installing locked dependencies (including nested sidecar installs)'
bun install --frozen-lockfile
bun run typecheck
bun run lint
bun run test
bash scripts/check-rust.sh

# Keep the original chain (these helpers build BOTH architectures, ignoring TARGET).
# Tauri then selects the target-suffixed sidecars and signs them with app entitlements.
if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  export BLUEY_CODESIGN_IDENTITY="$APPLE_SIGNING_IDENTITY"
fi
TARGET="$TARGET" bash scripts/build-helper.sh
BLUEY_AGENT_VARIANT="$AGENT_VARIANT" TARGET="$TARGET" bash scripts/build-agent.sh
for bin in bluey-helper bluey-agent; do
  [[ -x "src-tauri/binaries/${bin}-${TARGET}" ]] || fail 'missing target sidecar binary'
done

if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  # Explicit ad-hoc app signing for no-credential developer builds; never an eligible path.
  export APPLE_SIGNING_IDENTITY=-
  unset APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID
  printf '%s\n' 'Developer-only ad-hoc build; NOT eligible for publication.'
fi
# Config retains hardened runtime + entitlements.plist. No --no-sign/--skip-stapling,
# no recursive re-signing. Tauri notarizes + staples the app before making the DMG, then
# writes the signed updater bundle (`Bluey.app.tar.gz` + `.sig`) next to the app.
bun run tauri build --target "$TARGET" --bundles app,dmg "${TAURI_CONFIG_ARGS[@]}" -- --locked

# Tests/builds must not have changed dependency locks. There is no install fallback.
git diff --exit-code -- bun.lock sidecars/agent/bun.lock src-tauri/Cargo.lock
if [[ "$PUBLISH_RELEASE" == true ]]; then
  python3 scripts/release/verify_macos.py stage --root "$ROOT" --target "$DOWNLOAD_TARGET" \
    --tag "$RELEASE_TAG" --commit "$RELEASE_COMMIT" --run-id "$GITHUB_RUN_ID" \
    --run-attempt "$GITHUB_RUN_ATTEMPT" --output "$VERIFIED_DIR"
else
  printf '%s\n' 'Developer build complete (not a published release and no eligible verification record).'
fi
printf 'Bundle directory: %s\n' "$BUNDLE_DIR"
