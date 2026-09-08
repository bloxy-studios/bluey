#!/usr/bin/env bash
# Build the Bluey native helper (Swift sidecar) for both macOS architectures
# and install the binaries where Tauri's externalBin/sidecar lookup expects them:
#
#   ./scripts/build-helper.sh        # both macOS architectures (release / CI)
#   ./scripts/build-helper.sh host   # this machine only (`tauri dev`)
#
# End users never need Swift — these binaries ship inside the .app bundle.
# Developers building Bluey from source need Xcode (or the Command Line Tools)
# with a Swift 5.9+ toolchain on macOS 14+.
#
# Signing:
#   APPLE_SIGNING_IDENTITY set   → codesign with that identity (+ hardened runtime)
#   otherwise                    → ad-hoc signature ("-") with hardened runtime
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PKG_DIR="$REPO_ROOT/src-tauri/swift/BlueyHelper"
OUT_DIR="$REPO_ROOT/src-tauri/binaries"
ENTITLEMENTS="$PKG_DIR/bluey-helper.entitlements"
PRODUCT="bluey-helper"
CONFIG="release"

if ! command -v swift >/dev/null 2>&1; then
    cat >&2 <<'EOF'
error: `swift` not found.

Building the Bluey native helper requires a Swift toolchain (macOS only):
  1. Install Xcode or the Command Line Tools:  xcode-select --install
  2. Re-run:                                   ./scripts/build-helper.sh

Note: this is a *build-time* requirement only. End users run the prebuilt
bluey-helper binaries bundled inside Bluey.app and never need Swift.
EOF
    exit 1
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "error: the helper links macOS-only frameworks (ScreenCaptureKit, Vision, …) and can only be built on macOS." >&2
    exit 1
fi

mkdir -p "$OUT_DIR"

# arch → Rust/Tauri target-triple suffix
build_one() {
    local arch="$1" triple="$2"
    echo "==> swift build -c $CONFIG --arch $arch ($PRODUCT)"
    swift build --package-path "$PKG_DIR" -c "$CONFIG" --arch "$arch" --product "$PRODUCT"

    # Resolve the per-arch bin path from SwiftPM itself (robust across SwiftPM versions).
    local bin_path
    bin_path="$(swift build --package-path "$PKG_DIR" -c "$CONFIG" --arch "$arch" --product "$PRODUCT" --show-bin-path)"
    local src="$bin_path/$PRODUCT"
    if [[ ! -f "$src" ]]; then
        echo "error: built binary not found at $src" >&2
        exit 1
    fi

    local dest="$OUT_DIR/$PRODUCT-$triple"
    cp -f "$src" "$dest"

    # Strip debug info + local symbols (safe for a signed release executable).
    strip -Sx "$dest" || echo "warning: strip failed for $dest (continuing)" >&2

    # Code signing. Hardened runtime is required for notarization; the
    # audio-input entitlement is required for microphone access under the
    # hardened runtime (TCC prompts are attributed to the parent .app).
    local sign_args=(--force --options runtime)
    if [[ -f "$ENTITLEMENTS" ]]; then
        sign_args+=(--entitlements "$ENTITLEMENTS")
    fi
    if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
        echo "==> codesign ($APPLE_SIGNING_IDENTITY) $dest"
        codesign "${sign_args[@]}" --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$dest"
    else
        echo "==> codesign (ad-hoc) $dest"
        codesign "${sign_args[@]}" --sign - "$dest"
    fi

    echo "==> installed $dest"
}

if [[ "${1:-}" == "host" ]]; then
    case "$(uname -m)" in
        arm64)  build_one "arm64"  "aarch64-apple-darwin" ;;
        x86_64) build_one "x86_64" "x86_64-apple-darwin" ;;
        *) echo "error: unsupported host arch $(uname -m)" >&2; exit 1 ;;
    esac
else
    build_one "arm64" "aarch64-apple-darwin"
    build_one "x86_64" "x86_64-apple-darwin"
fi

echo "Done. Binaries:"
ls -la "$OUT_DIR" | grep "$PRODUCT" || true
