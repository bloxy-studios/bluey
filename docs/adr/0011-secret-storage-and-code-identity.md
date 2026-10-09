# ADR 0011 — Secret storage: attribute-only Keychain access, an in-process cache, and a stable code identity

**Status:** accepted · **Date:** 2026-09-28 · companion: `docs/SECURITY.md` (secret handling),
`docs/audits/BLUEY_DEEP_AUDIT_2026-09-28.md` §4, probe `docs/audits/2026-09-28/keychain-acl-probe.swift`

## Context
After every update (and after every `tauri dev` rebuild) macOS asked for the login password —
sometimes several times in a row — before Bluey could use its own saved API keys and sign-in.
The audit reproduced the cause with a probe against the legacy file keychain:

* **Code identity.** A Keychain item's access list trusts the *code identity* that created it.
  An ad-hoc or unsigned build is identified by its cdhash, which changes on every build. A
  Developer ID (or Apple Development) signature is identified by bundle identifier + Team ID,
  which survives rebuilds and updates. Bluey's developer and nightly builds were ad-hoc.
* **Access pattern.** Reading an item's *data* from an untrusted build returns
  `errSecAuthFailed` (-25293) — that is the login-password prompt. An *attribute-only* lookup
  (does the item exist? which accounts does the service hold?) never prompts. The `keyring`
  crate (v3) decrypted the item for `has`, `set` and even `delete`, so presence checks at boot,
  every settings save and every sign-out prompted.
* **Writes.** An in-place modify from an untrusted build can leave the item locked for every
  build. Deleting the item (found by attributes) and adding it again makes the running build
  its owner, without a prompt.
* **Volume.** Tokens were rewritten after every subscription request and every boot even when
  unchanged; presence flags were recomputed with decrypting reads under the settings lock.

## Decision
1. **One backend seam.** `secrets::backend::SecretBackend` has five operations — `read`
   (Present / Absent / Locked{status}), `exists` and `list` (attribute-only), `write` (attribute
   delete, then add; never an in-place modify) and `remove` (attribute-only, status checked).
   The production `KeychainBackend` calls `security-framework` directly on the same service and
   account names as before, so existing items keep working. Tests use `CountingFake`, which
   records every operation so a test can pin how many prompting reads an action costs.
2. **A cache in front of it.** `SecretsStore` keeps values (in `Zeroizing`) and a presence map
   for the life of the process, invalidated on set / delete / reset; errors are never cached.
   Boot seeds presence with one attribute-only enumeration, so `has_api_key`, the stored
   sign-in and research availability cost zero decrypting reads. A value is read at most once
   per key per process, on first use. Both allow-lists (`validate_key`, `validate_webview_key`)
   are unchanged.
3. **Locked is not absent.** Access denied (-25293, -128), interaction not allowed (-25308) and
   an unavailable keychain map to distinct codes (`storage.keychain_access_denied`,
   `storage.keychain_interaction_not_allowed`, `storage.keychain_unavailable`) with copy in
   `src/lib/errors/present.ts`. Presence stays tri-state
   (`present | locked | absent`); a locked key never shows as "no key" and never triggers a
   re-entry or a sign-out.
4. **Fewer writes.** Account tokens and the Clerk session are written only when a refresh
   actually changed them. Imported rotating sessions (Claude Code, Codex) are never refreshed
   by Bluey. Sign-out, disconnect and reset delete with attribute-only calls and always clear
   local state, even if the Keychain delete fails (reported as a warning).
5. **Visible state.** Settings → Privacy → *Saved credentials* lists every Bluey-owned item by
   name and state (never a value), computed with a non-interactive probe. *Allow access* is the
   single, deliberate interactive read.
6. **Separate, stable development identity.** Debug builds use the service
   `com.codewithabdul.bluey.dev`, so dev builds never touch the installed app's items. An opt-in
   cargo target runner (`scripts/dev-sign-runner.sh`) signs the dev binary with an Apple
   Development identity (`BLUEY_DEV_SIGNING_IDENTITY`); `scripts/release.sh` accepts
   `BLUEY_LOCAL_SIGNING_IDENTITY` for local, non-publishable builds.

## Consequences
* After an update of an ad-hoc build, Bluey still cannot read items created by the previous
  build without the user's approval — but it now asks at most once per item, only when the
  value is actually needed (or when the user clicks *Allow access*), and never at boot, on a
  settings save, on sign-out or on reset. *Always Allow* makes the answer stick for that build.
* The root fix is a stable code identity for everything users install: **Developer ID signing
  and notarization of every published build, including nightlies**. That is an owner action
  (Apple Developer Program membership and the release credentials in `docs/RELEASING.md`); with
  it, updates keep the same identity and never re-prompt.
* A self-signed certificate does **not** help: macOS pins items created by such a build to its
  cdhash, exactly like ad-hoc. Local stable signing needs an Apple-issued (Team ID) identity.
* Consolidating every Bluey secret into one "vault" item (one prompt instead of N after an
  identity change) was considered and deferred: it enlarges the blast radius of a corrupt item
  and is unnecessary once published builds are Developer ID signed.

## Alternatives considered
* **Keep `keyring`** — its macOS store decrypts on `has`/`set`/`delete`; upgrading does not
  change the access pattern for the legacy file keychain.
* **Data-protection keychain** (`kSecUseDataProtectionKeychain`) — requires a provisioning
  profile / keychain-access-group entitlement, so it is unavailable to ad-hoc and dev builds,
  and existing items would need a migration that prompts anyway.
* **An encrypted file under Application Support** — moves key management into Bluey and loses
  the Keychain's per-app access control; rejected (ADR 0001 keeps secrets in the Keychain).
