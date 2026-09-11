#!/usr/bin/env bash
# Type-check and test the Rust side.
#
#   scripts/check-rust.sh            # host tests for the platform-independent crates
#   scripts/check-rust.sh --darwin   # additionally `cargo check` the Tauri app crate for
#                                    # aarch64-apple-darwin (works from Linux CI too: no linking)
#
# On a Linux host the darwin check uses scripts/fake-darwin-cc.sh as the C compiler for
# `cc`-based build scripts (objc2-exception-helper, bundled sqlite). `cargo check` never
# links, so an empty object file is enough. Never use this trick for real builds.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
cd "$ROOT/src-tauri"

echo "▶ cargo fmt --check"
cargo fmt --all -- --check
echo "▶ cargo test (bluey-core, bluey-storage, bluey-protocols, bluey-oauth)"
cargo test -p bluey-core -p bluey-storage -p bluey-protocols -p bluey-oauth
echo "▶ cargo clippy (bluey-core, bluey-storage, bluey-protocols, bluey-oauth)"
cargo clippy -p bluey-core -p bluey-storage -p bluey-protocols -p bluey-oauth --all-targets -- -D warnings

if [[ "${1:-}" == "--darwin" ]]; then
  echo "▶ cargo check --target aarch64-apple-darwin (app crate)"
  if [[ "$(uname -s)" != "Darwin" ]]; then
    export CC_aarch64_apple_darwin="$ROOT/scripts/fake-darwin-cc.sh"
    export CXX_aarch64_apple_darwin="$ROOT/scripts/fake-darwin-cc.sh"
    export AR_aarch64_apple_darwin=ar
    # Separate target dir so the cross-check never holds the host build lock.
    export CARGO_TARGET_DIR="$ROOT/src-tauri/target-darwin"
    rustup target add aarch64-apple-darwin >/dev/null 2>&1 || true
    # The bundled SQLite C library cannot be cross-compiled here; type-check without it.
    cargo check --target aarch64-apple-darwin --no-default-features --features dev-tools
  else
    cargo check --target aarch64-apple-darwin
  fi
fi
echo "✓ rust checks passed"
