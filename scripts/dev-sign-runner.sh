#!/usr/bin/env bash
# Cargo target runner for macOS (src-tauri/.cargo/config.toml). `tauri dev`
# starts the app with `cargo run`, so every dev build passes through here.
#
# Opt-in stable code identity for development builds (ADR 0011): with
# BLUEY_DEV_SIGNING_IDENTITY set, the `bluey` app binary is signed with that
# identity and the fixed identifier com.codewithabdul.bluey.dev before it
# starts, so the dev Keychain items and privacy permissions survive rebuilds
# instead of asking again after every `cargo build`.
#
#   BLUEY_DEV_SIGNING_IDENTITY=auto        first "Apple Development" identity
#   BLUEY_DEV_SIGNING_IDENTITY="Apple Development: Jane Doe (ABCDE12345)"
#   BLUEY_DEV_SIGNING_IDENTITY=<SHA-1 of the certificate>
#
# It must be an Apple-issued identity (Team ID). A self-signed certificate
# does not help: macOS pins those to the binary's hash, exactly like ad-hoc.
# Unset (the default), or for any other binary (cargo test harnesses), the
# program is exec'd untouched.
set -euo pipefail

BIN="${1:?usage: dev-sign-runner.sh <binary> [args...]}"
IDENTITY="${BLUEY_DEV_SIGNING_IDENTITY:-}"

if [[ -z "$IDENTITY" || "$(basename "$BIN")" != "bluey" ]]; then
  exec "$@"
fi

if [[ "$IDENTITY" == "auto" ]]; then
  IDENTITY="$(security find-identity -v -p codesigning |
    awk '/"Apple Development/ { print $2; exit }')"
  if [[ -z "$IDENTITY" ]]; then
    echo "dev-sign-runner: no 'Apple Development' identity in the keychain;" \
      "running $BIN unsigned (see docs/DEVELOPMENT.md)" >&2
    exec "$@"
  fi
fi

if ! codesign --force --sign "$IDENTITY" --identifier com.codewithabdul.bluey.dev \
  "$BIN"; then
  echo "dev-sign-runner: codesign with '$IDENTITY' failed; running $BIN unsigned" >&2
fi
exec "$@"
