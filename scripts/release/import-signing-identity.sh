#!/usr/bin/env bash
# Import a stable code-signing identity for nightly builds (optional).
#
# macOS ties Screen Recording, Microphone, Accessibility, Speech Recognition and Keychain
# approvals to the app's code identity. An ad-hoc nightly is a new identity on every build,
# so each update asks for all of them again (ADR 0011, docs/UPDATES.md). Signing every
# nightly with one certificate — the owner's Apple Development certificate is enough — keeps
# that identity, and the approvals, across updates. Nightlies stay developer builds: never
# notarized, never eligible for publication (scripts/release.sh).
#
# Inputs (step-level env, from repository secrets):
#   CERT_P12_BASE64  base64 of the exported certificate + private key (.p12)
#   CERT_PASSWORD    the .p12 export password
# Without them the build stays ad-hoc, as before. Writes BLUEY_LOCAL_SIGNING_IDENTITY to
# $GITHUB_ENV for scripts/release.sh. The keychain lives in $RUNNER_TEMP on a disposable runner.
set -euo pipefail

fail() { printf 'import-signing-identity: %s\n' "$1" >&2; exit 1; }

if [[ -z "${CERT_P12_BASE64:-}" ]]; then
  printf '%s\n' 'No stable signing identity configured: this nightly is signed ad hoc.'
  exit 0
fi
[[ -n "${CERT_PASSWORD:-}" ]] || fail 'the certificate password secret is missing'
[[ -n "${RUNNER_TEMP:-}" && -n "${GITHUB_ENV:-}" ]] || fail 'runs only inside GitHub Actions'

keychain="$RUNNER_TEMP/bluey-nightly-signing.keychain-db"
p12="$RUNNER_TEMP/bluey-nightly-signing.p12"
keychain_password="$(openssl rand -hex 24)"
trap 'rm -f "$p12"' EXIT

printf '%s' "$CERT_P12_BASE64" | base64 --decode > "$p12" || fail 'the certificate is not valid base64'
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$p12" -k "$keychain" -P "$CERT_PASSWORD" -T /usr/bin/codesign -T /usr/bin/security \
  >/dev/null || fail 'the certificate could not be imported (wrong password?)'
# Let codesign use the key without a UI prompt.
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null
# Put the keychain on the search list next to the runner's own.
existing="$(security list-keychains -d user | sed -e 's/^[[:space:]]*"//' -e 's/"$//')"
# shellcheck disable=SC2086 # one path per line, none with spaces on a hosted runner
security list-keychains -d user -s "$keychain" $existing

# The certificate's SHA-1 names the identity unambiguously (no name parsing).
identity="$(security find-identity -v -p codesigning "$keychain" | awk '/^ *1\)/ { print $2; exit }')"
[[ "$identity" =~ ^[0-9A-F]{40}$ ]] || fail 'no valid code-signing identity in the certificate'
printf 'BLUEY_LOCAL_SIGNING_IDENTITY=%s\n' "$identity" >> "$GITHUB_ENV"
printf '%s\n' 'Nightly will be signed with the stable identity from the repository secrets.'
