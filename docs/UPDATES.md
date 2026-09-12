# In-app updates

Bluey updates itself. A build follows one of two **channels**, checks its channel's feed in the
background, and — with *Automatic updates* on (the default) — downloads, verifies and installs
the new version by itself; the user only decides when to relaunch. Everything the user sees is
in **Settings → General → Updates** and the HUD's left pill.

## Channels and feeds

| Channel | Follows | Feed the app reads |
|---|---|---|
| **Latest** (default) | stable GitHub Releases (`v1.2.3`) | `https://github.com/bloxy-studios/bluey/releases/latest/download/latest.json` |
| **Nightly** | the rolling `nightly` prerelease built from `main` every night `main` changed | `https://github.com/bloxy-studios/bluey/releases/download/nightly/latest.json` |

Both feeds are static `latest.json` files in Tauri's updater format —
`version`, `notes`, `pub_date`, `platforms.darwin-aarch64.{url,signature}` and
`platforms.darwin-x86_64.{url,signature}` — produced by the release pipeline
(`docs/RELEASING.md › Updater artifacts`). GitHub resolves `releases/latest/download/…` to the
newest **non-prerelease** release, so the Latest channel never sees a nightly or an `-rc`.

Nightly versions look like `0.1.2-nightly.20260913`: SemVer-greater than the current stable, smaller
than the next stable. A user who switches from Nightly back to Latest is offered the next stable
when it ships; a Latest user is never offered a nightly.

## Signing

Tauri's updater refuses any bundle whose **minisign** signature does not verify against the
public key compiled into `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`). This is
independent of Apple code-signing and works for unsigned developer builds too.

- The private key and its password are the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; `tauri build` signs `Bluey.app.tar.gz` with them when
  `bundle.createUpdaterArtifacts` is on. The owner keeps a backup of both: **losing the key means
  installed apps can never update again** (they would need a fresh download).
- Rotation: generate a new pair (`bun run tauri signer generate -w ~/.tauri/bluey-updater.key`),
  ship one release signed with the **old** key whose config carries the **new** public key, then
  switch CI to the new key.

## What the app does

`crate::updates::UpdatesManager` (app crate) owns the cycle and publishes every transition as the
`update.status` event; the WebView's `updatesStore` mirrors it.

| Phase | Meaning | HUD pill | Settings row button |
|---|---|---|---|
| `idle` | nothing known yet | — | Check now |
| `checking` | feed request in flight | — | Checking… (disabled) |
| `up_to_date` | feed version ≤ current | — | Check now |
| `available` | newer version found | **Update available · 0.2.0** (click = Install) | Install |
| `downloading` | download + install in progress | **Updating… 42%** | Updating… (disabled) |
| `ready` | installed on disk | **Restart to update** (click = relaunch) | Restart to update |
| `error` | check or install failed | — (row shows the reason) | Check now |

- **Schedule**: one check 30 s after launch, then every 6 hours; a channel change checks
  immediately. A `ready` update is never re-checked.
- **Automatic updates** (default on): `available` moves straight on to `downloading` and `ready`.
  Off: the pill and the row offer *Install*. Turning it on while an update is `available` installs it.
- The pill only appears when the HUD is otherwise idle — never over Listening, Thinking or a
  prepared suggestion (`derivePill`, `src/features/hud/state-pill.ts`).
- The current version keeps running through every failure; errors are `update.check_failed`
  and `update.install_failed` (copy in `src/lib/errors/present.ts`), shown in the Settings row.
- **Debug builds** (`tauri dev`) report `supported: false`: a manual check answers, nothing is
  installed and no background check runs. The browser mock simulates the whole cycle (it always
  "finds" `0.2.0`, or `0.2.0-nightly.…` on the Nightly channel).

## Contracts

- Settings: `settings.updates = { channel: "latest" | "nightly", automatic: boolean }`
  (`bluey_core::types::UpdatesSettings` ⇄ `UpdatesSettings` in `src/lib/types/settings.ts`; absent in
  older stores → defaults).
- Status: `bluey_core::types::UpdateStatus` ⇄ `UpdateStatus` in `src/lib/types/updates.ts`.
- Commands: `updates_get_status`, `updates_check`, `updates_install`, `updates_relaunch`
  (`bluey.updates.*`). Event: `update.status`.

## First updatable build

Releases before 0.1.2 have no updater; that generation downloads 0.1.2 by hand once from the
download page. From then on every release — stable or nightly — reaches installed apps through
the channel they follow.
