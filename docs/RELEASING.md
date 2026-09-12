# macOS releases

This is the owner runbook for `.github/workflows/release.yml`. The workflow supports
**macOS Apple silicon and Intel only**, as two DMGs plus the two minisign-signed in-app updater
bundles and their `latest.json` feed (`docs/UPDATES.md`). It does not implement a Windows/Linux
port or a universal binary. The only release built from a branch is the rolling `nightly`
prerelease (`nightly.yml`, see *Updater artifacts*), which never touches a version tag. Nothing
in the local implementation/review runs creates a tag, invokes a cloud build or publishes a release.

## Two deliberately separate paths

- **Developer build (default manual dispatch):** `publish=false`, `release_tag` empty. Builds
  `.app` + `.dmg` for both targets and uploads `developer-*` Actions artifacts, without signing
  secrets. These are ad-hoc/developer bundles, not distributable installers. The publish job
  cannot consume them. `research_backend=gemini` (default/lite) and `claude` (full embedded CLI)
  remain available.
- **Publication:** a `v*` tag push, or explicit `publish=true` with an **existing exact version
  tag**. Missing/partial credentials, version mismatch, changed tag, failed build, failed Apple
  checks, missing target or an existing release (including a draft) stop publication. No
  unsigned fallback exists on this path.

Locally on macOS, the original build-only entry point remains:

```bash
# Bun must be exactly 1.4.2; macOS 14+, Xcode CLI tools, Python 3.9+, stable Rust are required.
TARGET=aarch64-apple-darwin RESEARCH_BACKEND=gemini bash scripts/release.sh
TARGET=x86_64-apple-darwin RESEARCH_BACKEND=claude bash scripts/release.sh
```

Omit `TARGET` to use `rustc --print host-tuple`. Local installed signing identities and Apple
notarization inputs can still be used for build-only bundles, but this never creates the
publication-eligible record or a GitHub Release. Do not set `PUBLISH_RELEASE=true` locally;
that is an internal, gated Actions path, not a shortcut to publish arbitrary files.

## Owner setup (required before first publication)

1. Merge/review the release workflow **and its Python/shell helpers and tests** through the
   normal repository process. Version tags must contain those files. Protect the default
   branch and review changes to `.github/workflows/`, `scripts/release*`, signing configuration,
   dependency locks and build hooks. Do not dispatch release jobs for unreviewed code.
2. In GitHub, create the **`macos-release` environment** with required owner reviewers and
   restricted deployments for protected version tags. Set a tag ruleset for `v*` that limits
   creation to release owners and **prevents tag updates/deletion**. Do not allow tag-force or
   deletion bypasses in routine publishing. Enable GitHub immutable releases where available.
   API existence checks cannot atomically prevent a privileged actor deleting/moving a tag
   between requests; tag protection is an essential owner-side prerequisite.
3. Put the six required values below in `macos-release` environment secrets. Never use a
   pull-request workflow, public variables, command arguments typed into logs, or checked-in
   `.env` files for credentials. The optional `macos-build` environment must contain **no
   signing secrets**; build-only dispatches don't receive the signing values in any case.
4. Confirm Actions is permitted to create Releases using `GITHUB_TOKEN`. The workflow defaults
   to `contents: read`; only the `publish` job has `contents: write`. Checkout never persists
   credentials. No PAT is required. There is no `pull_request`/`pull_request_target` release
   trigger and no cross-run build cache on the signing path.
5. Configure the intended public Clerk identifiers in environment/repository **variables**
   (next section), then perform native acceptance on owner-controlled Macs. This change was
   implemented/tested on Linux and is **not evidence of a working native release**.
6. Store the updater signing key as the **repository** secrets `TAURI_SIGNING_PRIVATE_KEY` and
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (every build path — publication, developer, nightly —
   signs its updater bundle, so an environment-scoped copy is not enough) and keep an offline
   backup: a lost key strands every installed app on its current version (`docs/UPDATES.md ›
   Signing`). The public key lives in `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`).

| Required environment secret | Purpose |
| --- | --- |
| `APPLE_CERTIFICATE_P12` | Base64 export of a **Developer ID Application** certificate **with its private key**, not a Mac App Store/installer certificate |
| `APPLE_CERTIFICATE_PASSWORD` | Nonempty password protecting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | Exact valid identity, `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID` | Apple developer account permitted to notarize for this team |
| `APPLE_PASSWORD` | App-specific notarization password, not the normal account password |
| `APPLE_TEAM_ID` | Matching ten-character Apple team ID |

Presence, identity/team shape and base64 certificate shape are checked before installing Bun
or a Rust toolchain/building. Import then verifies the configured valid codesigning identity
actually exists in the temporary keychain. Apple authentication, certificate trust/expiry and
notarization acceptance are checked by the real tools, not inferred from environment presence.
A randomly password-protected temporary keychain allows `codesign` access, restores the
original search list on cleanup, and deletes its `.p12` immediately after import. The cleanup
step runs even after build failure. Use disposable hosted runners: hard cancellation/runner
loss can interrupt cleanup. Do not enable shell tracing or verbose environment/build dumps.

### Public runtime configuration / optional OAuth value

The workflow exposes only the existing build-time public identifier allowlist:
`VITE_CLERK_PUBLISHABLE_KEY`, `VITE_CLERK_FRONTEND_API_URL`, `BLUEY_CLERK_OAUTH_CLIENT_ID`,
`BLUEY_CLERK_ACCOUNT_PORTAL_URL`. Configure the correct Clerk public PKCE app/redirects as
explained in [Development](DEVELOPMENT.md). An empty value does not fail signing but can leave
sign-in unconfigured; verify the installed app's sign-in before approving a release.

`BLUEY_ANTIGRAVITY_CLIENT_SECRET` is **optional**. The existing `src-tauri/build.rs` supports
baking this desktop OAuth client value into the binary; it is not a user/provider API key and
cannot be kept confidential once compiled into a desktop app. If the owner elects to supply
it, store it as a **repository** secret (or in both the `macos-build` and `macos-release`
environments): unlike the signing values, the workflow passes it to **both** paths, so developer
build-only bundles bake it as well — otherwise the shipped app's Google AI card reports that this
build cannot sign in. It is not a signing credential and its presence never gates publication.
The release scripts never print it or require it. If absent, the relevant Google
subscription-account connection may remain unavailable; do not block a macOS release or invent a
value for it. No Gemini/Anthropic/OpenAI keys, Clerk secret
keys or other user credentials are required for builds/tests. No `.env` was read or changed
as part of this release-workflow implementation.

## Version/tag and output audit

At the audited source commit `a4e3fb84178f890ba12c3a5fdac73cd065718dff`, actual reads show:

| Source entry | Value |
| --- | --- |
| `package.json` → `version` | `0.1.0` |
| `src-tauri/tauri.conf.json` → `version` | `0.1.0` |
| workspace-root `src-tauri/Cargo.toml` → `[package].version` | `0.1.0` |
| `[workspace.package].version` | **Not present** (do not invent a fourth version) |
| `bundle.macOS.minimumSystemVersion` | `14.0` |
| `productName` / identifier | `Bluey` / `com.codewithabdul.bluey` |

Every publication reads these source files anew. Root package versions must all equal the
tag with an optional leading `v` removed. If a workspace version is later introduced, it is
also checked; `version.workspace = true` is supported. Private workspace member libraries
are not implicitly versioned together. The Python 3.9-compatible Cargo version reader is
intentionally small and fail-closed, accepting canonical single-line literal version entries;
multiline TOML or alternate version-table syntax requires an explicit reader review.

SemVer prerelease identifiers (e.g. `v0.2.0-rc.1`) produce a **prerelease**, not stable/latest.
Build metadata alone (e.g. `v0.2.0+build.2`) is not a prerelease. Stable finalization uses
GitHub's `make_latest=legacy` version/date selection rather than forcibly making an older
backfill latest. The tag must already resolve to the checked-out commit; preflight records
that full SHA and both matrix builds/publisher check it again. The remote tag is checked
before draft creation and again before finalization. No tag is created, changed or force-pushed
by these scripts.

The lock pins Tauri CLI **2.11.4**. Its audited bundler source:
[app signing/notarization](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/macos/app.rs),
[DMG creation/signing](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/macos/dmg/mod.rs),
[entitlements/signing](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/macos/sign.rs).
It signs/notarizes/staples the app before DMG creation, and signs (but does not itself notarize)
the DMG. The release verifier therefore explicitly submits the resulting DMG with
`xcrun notarytool --wait`, requires JSON `status=Accepted`, and staples it.

- `CARGO_TARGET_DIR` is explicitly `src-tauri/target`; per-target outputs are under
  `<target>/release/bundle/macos/*.app`, `<target>/release/bundle/dmg/*.dmg` and — because
  `bundle.createUpdaterArtifacts` is on — `<target>/release/bundle/macos/*.app.tar.gz` with its
  `.sig` (base64 minisign signature made with `TAURI_SIGNING_PRIVATE_KEY`).
- Old target bundle directories are removed before building, and the script itself fails when
  the build leaves no DMG, updater archive or signature — a build cannot look green without its
  outputs (macOS runs `release.sh` under bash 3.2; the script stays free of 3.2 pitfalls such as
  empty-array expansion under `set -u`). Exactly **one** app, **one** DMG,
  **one** updater archive and **one** signature are discovered per target. Real DMG basenames are
  preserved, never synthesized from a filename example or used to infer the architecture; the
  updater archive (always `Bluey.app.tar.gz` as Tauri writes it) is staged under the DMG's stem
  (`Bluey_1.2.3_aarch64.app.tar.gz`) so the two targets' bundles have distinct asset names.
- `build-helper.sh` and `build-agent.sh` actually build **both architectures** unless given
  `host`; they do **not** honor `TARGET` as a build selector. The release script retains this
  existing chain; Tauri selects `binaries/bluey-helper-<target>` and `bluey-agent-<target>`.
  In the bundled app their names are `Contents/MacOS/bluey-helper` and `bluey-agent`.
- Existing app `entitlements.plist` and Swift helper microphone entitlements remain untouched.
  Tauri's signing applies app entitlements to embedded binaries. The verifier checks the app's
  configured entitlements, helper microphone entitlement and agent JIT entitlement. It never
  recursively re-signs a bundle or strips capabilities. Every bundled executable's `lipo`
  architecture must equal the target, including both sidecars.
- Bun is exactly **1.4.2**. Root and sidecar installs are frozen. A release-only PATH shim
  adds `--frozen-lockfile` to the unchanged agent helper's nested `bun install`, including the
  full variant's `--os darwin --cpu '*'`. No fallback install exists. Tauri's Cargo build uses
  `--locked`, and lock drift after checks/build fails before staging.
- Automatic `tauri.macos.conf.*`/`TAURI_CONFIG` overrides, changed signing paths, disabled
  hardened runtime/stapling or unreviewed external-bin layouts fail closed. The single permitted
  `--config` override is the nightly version (`BLUEY_BUILD_VERSION`, strictly
  `X.Y.Z-nightly.YYYYMMDD`), and only on the developer path — `PUBLISH_RELEASE=true` refuses it.

## Updater artifacts

Every release carries what the in-app updater needs (`docs/UPDATES.md`):

- `Bluey_<version>_aarch64.app.tar.gz` + `.sig` and `Bluey_<version>_x64.app.tar.gz` + `.sig`:
  the signed app bundles Tauri built, verified exactly like the DMG's embedded app — the
  verifier extracts each archive (members must stay inside the bundle; exactly one `*.app`)
  and runs the same signature/notarization/entitlement/architecture checks (`updaterApp`).
- `latest.json`: Tauri's static feed — `version`, `notes`, `pub_date` (RFC 3339), and
  `platforms.darwin-aarch64` / `platforms.darwin-x86_64` each with the archive's
  `https://github.com/bloxy-studios/bluey/releases/download/<tag>/<asset>` URL and its
  signature text. It is generated from the verified bundles (never hand-written), validated
  against them again before upload, and uploaded **after** the archives so it never points at
  an asset that does not exist yet. `SHA256SUMS` covers the DMGs and the archives.

The **Latest** channel reads `releases/latest/download/latest.json`, which GitHub resolves to the
newest non-prerelease release; publishing a stable release therefore updates every installed app
on that channel. The **Nightly** channel reads the rolling `nightly` prerelease, maintained by
`.github/workflows/nightly.yml` (mirror in `docs/ci/workflows/`):

1. `plan` (ubuntu, read-only) runs the portable tests, refuses any ref but `main`, computes
   `X.Y.(Z+1)-nightly.YYYYMMDD` from the sources, and reads the last nightly's commit from the
   marker `<!-- bluey-nightly commit=… version=… -->` in the release body. Unchanged `main`
   skips the night unless dispatched with `force`.
2. `build` (macos-14 matrix, `macos-build` environment): the ordinary developer path of
   `scripts/release.sh` — ad-hoc Apple signature, **no Apple secrets**, minisign-signed updater
   bundle — with `BLUEY_BUILD_VERSION` overriding the version. Nightlies are therefore
   unsigned/un-notarized builds until Apple credentials exist; the release body says so.
3. `publish` (ubuntu, the only `contents: write` job): creates the `nightly` prerelease on the
   first run, otherwise force-moves the `nightly` tag to the built commit; deletes and re-uploads
   `SHA256SUMS`/`latest.json` (and same-day reruns' bundles), uploads the DMGs, archives and
   signatures, reads every stored asset back, uploads the feed **last**, then removes the previous
   night's bundles and rewrites the body/marker. `make_latest` is always `false`, so a nightly
   never becomes the Latest channel's release. The `v*` tag ruleset does not cover `nightly`;
   that tag is *meant* to move.

Local developer builds need the key too: `export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/bluey-updater.key)"`
(and its password) from the owner's backup, or a throwaway pair from `bun run tauri signer generate`
for builds that will never feed real installs. Without either, `scripts/release.sh` stops before
installing anything.

## Publication transaction and installer contract

1. Resolve the request, source versions, existing local/remote tag and commit. A publishing
   dispatch must run from the exact requested tag, with the resolved tag commit bound to
   the triggering event SHA. The read-only preparation token rejects existing published
   releases; it may not see drafts. The write-authorized publisher performs the authoritative
   draft-or-published check again before creating anything.
2. Check/import every required credential before toolchain setup/build in each matrix job.
3. Build with frozen dependencies, retaining frontend checks, Rust checks, Swift helper and
   Bun agent build chain. Failed builds cannot upload eligible artifacts.
4. Check the built app's identifier/version/minimum OS, Developer ID/team/timestamp/hardened
   runtime, source entitlements, sidecar signatures and architectures. Run actual
   `codesign --verify --deep --strict`, `xcrun stapler validate`, and `spctl --assess`.
5. Notarize/staple the signed DMG, verify its signature/container/Gatekeeper/staple, mount it
   read-only, and repeat app/sidecar checks on the **embedded app actually distributed**; then
   extract the updater archive and repeat them on **the app the updater will install**.
   Only after all checks does an atomic directory rename expose DMG, updater archive + `.sig`
   and `verified.json`.
6. After **both matrix jobs succeed**, download only the exact two artifacts named with the
   current `github.run_id` **and `github.run_attempt`**. No wildcard, cross-run or developer
   artifact is accepted. Validate the provenance record and recompute sizes/SHA-256. The
   record is an internal assertion of the trusted workflow, **not a cryptographic attestation**.
   In the publish job, repeat native DMG + embedded-app verification independently after
   artifact transport. Hashes detect changes; Apple trust checks do not by themselves prove
   correspondence to a source commit. Protect workflow/runner/maintainer access accordingly.
7. Generate `bluey-downloads.json` (UTF-8, ≤64 KiB), `latest.json` and `SHA256SUMS` from the
   actual installers and updater bundles. Require exactly `mac-arm64` + `mac-x64`, both `dmg`,
   unique safe basenames, positive integer sizes, lowercase SHA-256, and minimum OS from
   source/verified app metadata. The manifest has exactly `schemaVersion: 1`, `version`, `tag`,
   `repository: "bloxy-studios/bluey"`, `installers`; each installer has `target`, `format`,
   `asset`, `bytes`, `sha256`, `minimumOsVersion`. There are no manifest download URLs and no
   source ZIP fallback. `SHA256SUMS` covers both DMGs and both updater archives; `latest.json`
   is described under *Updater artifacts*.
8. Recheck remote tag/no existing release, create **one new DRAFT**, upload both DMGs, both
   updater archives and signatures, checksums, the updater feed and finally the manifest, and
   download/hash each stored asset back. Recheck the draft's identity and exact asset set/state/
   size before a single final stable/prerelease PATCH. No overwrite, `--clobber`, DELETE, resume
   of another draft, or force-tag action is implemented on this path.

The downloadable release consists of exactly nine assets: two DMGs, two `.app.tar.gz` updater
bundles with their two `.sig` files, `SHA256SUMS`, `latest.json` and `bluey-downloads.json`. The
site matches real asset names, sizes and uploaded state to the manifest; examples are never
authoritative filenames. Developer Actions artifacts and `verified.json` are not site-facing
release assets.

## Manual owner release procedure

- Review and update the three application version entries together (and workspace version if
  one is introduced). Run the portable checks below and native acceptance. Owner tooling may
  create an annotated/signed, protected version tag **once** at the approved commit; this
  workflow never creates a missing tag. Pushing a `v*` tag requests publication automatically.
- To intentionally publish an existing tag (for example after configuration was fixed), use
  Actions → Release → Run workflow, **select that version tag as the workflow ref**, enable
  `publish`, put the same exact value in `release_tag`, and select the research backend.
  Equivalent owner-only command, **not run during implementation**:

  ```bash
  gh workflow run release.yml --repo bloxy-studios/bluey --ref v0.1.0 \
    -f publish=true -f release_tag=v0.1.0 -f research_backend=gemini
  ```

  Replace the example tag with the existing reviewed tag. The workflow file/helpers must be
  present at that tag. Environment deployment rules evaluate the workflow ref, not just the
  later checkout; selecting the tag avoids needing a branch deployment exception.
- Approve the protected environment jobs only after checking the tag, resolved SHA and intended
  backend. Wait for both builds and the publish job. Verify the final asset manifest/checksums
  and both actual installers before announcing availability.
- A manual build-only dispatch keeps `publish=false` and `release_tag` empty. Do not distribute
  those artifacts as signed public downloads or manually attach them to a Release.

### Failures / replay / partial upload

Concurrency serializes requests for the same tag and never cancels a publishing run to start
another. Once a release exists, even as a partial draft, publication fails rather than reusing
or clobbering it. Read-only preparation may not see a draft, so a rerun can repeat expensive
builds before the write-authorized publisher rejects it. Inspect and resolve partial drafts
before rerunning. If finalization times out, it may already have succeeded server-side:
**inspect GitHub first**. Never retry by overwriting assets of a published release.

If failure happened before draft creation, choose **Re-run all jobs**, not “Re-run failed jobs”
or only the publisher: current-attempt artifact names intentionally reject successful artifacts
from a prior attempt. If a partial draft remains, the owner must inspect the failed run and
all assets, verify the release is still a draft, and decide in the GitHub UI whether to remove
that incomplete **draft only** before re-running all jobs. Do not delete/recreate the tag.
Published mistakes require a new version/tag, not in-place mutation. Stale local staging
folders are rejected; use fresh disposable runners.

## Checks and explicit validation boundary

Portable, offline tests (Python standard library, Python 3.9+):

```bash
python3 -m unittest discover -s scripts/release/tests -v
python3 scripts/release/release_metadata.py version --root . --tag v0.1.0
bash -n scripts/release.sh scripts/release/bun-frozen.sh
# Optional when already installed; do not need a native build for this:
actionlint .github/workflows/release.yml .github/workflows/ci.yml
```

Use the current version instead of the example in the metadata command. Tests cover manifest
schema/names/bytes/hash/target-pair validation, version/tag/workspace mismatches, malicious
arguments, stale attempts, all credential presence gates, mock API draft/upload/finalization
ordering, failed uploads/readbacks/replays, mock native rejection propagation and bounded shell
mock builds/frozen installs. Shell tests substitute Bun/git/uname/helpers only inside temporary
fixtures; Apple tests mock commands and use fake file bytes. They **cannot prove** Apple
certificate acceptance, real signing, notary service behavior, correct hardened runtime at
launch, or an installable DMG. The optional CI job executes these portable tests without secrets
on PRs; release jobs never run there.

Before the first real release, an owner must verify on macOS with genuine credentials:

- Xcode/Swift/Rust/Bun builds for **both targets** and both intended backend variants as needed;
  actual valid certificate import, notarization account/team authorization and successful
  `codesign`, `notarytool`, `stapler`, `spctl`, `hdiutil` and `lipo` gates above.
- Both signed apps install/launch from their DMGs on matching **Apple silicon and Intel Macs**
  at the documented deployment floor; also test a fresh quarantined download, offline stapled
  launch, sidecar spawn/research, microphone/screen/accessibility permissions and configured
  Clerk sign-in. Cross-architecture signature checking on one runner is not a launch test.
- Native notarization accepts the chosen prerelease version representation in the app's
  Info.plist (Tauri writes its version string); do not weaken verification if Apple rejects it.
- GitHub environment/tag rules, read/write permissions, artifact transport and draft asset
  readback/finalization work as intended. No actual workflow/tag/Release was created in the
  Linux implementation session, so these owner/native integrations remain unverified.
