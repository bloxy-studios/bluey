# Development

## Prerequisites (macOS)

- macOS 14+, Xcode Command Line Tools (`xcode-select --install`) — provides Swift 5.9+.
- [Bun](https://bun.sh) ≥ 1.2, [Rust](https://rustup.rs) stable (≥ 1.85) with
  `rustup target add aarch64-apple-darwin x86_64-apple-darwin`.
- A Clerk application (publishable key) with a **public OAuth application** for Bluey
  (ADR 0008): Clerk Dashboard → *OAuth applications* → create, tick **Public** (PKCE), scopes
  `openid profile email offline_access`, redirect URIs `bluey://auth/callback` and
  `http://127.0.0.1/callback`; put its client id in `BLUEY_CLERK_OAUTH_CLIENT_ID`. Keep the
  consent screen on. Sign-in then happens in your default browser.
- A Google AI Studio key (`GEMINI_API_KEY`, free tier is fine) — the default provider for chat,
  vision, transcription, embeddings and research. Optional alternates: Microsoft Foundry / Azure
  OpenAI, Anthropic or an OpenAI-compatible endpoint; Exa / Firecrawl keys for research tools.

## Setup

```bash
bun install
cp .env.example .env            # or .env.local — VITE_CLERK_PUBLISHABLE_KEY + BLUEY_CLERK_OAUTH_CLIENT_ID + GEMINI_API_KEY (the rest is optional)
bun run tauri:dev               # builds missing sidecars on first run, then Vite + Rust + the app
```

### Where the environment comes from

- At startup the Rust backend loads `.env.local` and then `.env` — an earlier file wins, the
  process environment wins over both, empty assignments are ignored — from the repository root
  and `src-tauri` (development builds), then the current directory and the directory of the
  executable. This loader is the only way these files reach the backend: the Tauri CLI runs the
  app from `src-tauri`, `bun run` does not pass `.env` files to the scripts it starts, and Vite
  only exposes `VITE_*` to the WebView bundle. The log shows `loaded env file` with each path.
- The public Clerk settings (`VITE_CLERK_PUBLISHABLE_KEY`, `VITE_CLERK_FRONTEND_API_URL`,
  `BLUEY_CLERK_OAUTH_CLIENT_ID`, `BLUEY_CLERK_ACCOUNT_PORTAL_URL`) are additionally compiled
  into the binary by `src-tauri/build.rs` from the same files (the build environment wins over
  them) — the Rust-side equivalent of Vite baking `VITE_*` into the bundle — so `tauri build`
  products are configured without a file next to the app. Nothing else is ever compiled in; API
  keys stay in the Keychain and the runtime environment. Editing one of the files rebuilds the app
  crate automatically; after *creating* one, run `touch src-tauri/build.rs` once.

Other commands:

| Command                                | What                                                                                                                                                                                                  |
| -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `bun run dev`                          | Vite only — the UI in a browser with the **MockTransport** (Developer Mode backend): `http://localhost:1420/?window=main`, `?window=settings`, `?window=onboarding`, add `&dev=1` for the dev overlay |
| `bun run typecheck`                    | `tsc --noEmit` for the app and the agent sidecar                                                                                                                                                      |
| `bun run lint` / `bun run format`      | ESLint (strict TS, hooks) / Prettier                                                                                                                                                                  |
| `bun run test`                         | Vitest (unit, integration, UI)                                                                                                                                                                        |
| `bun run test:rust`                    | `cargo test` for `bluey-core`, `bluey-storage`, `bluey-protocols`, `bluey-oauth` and `bluey-fingerprints`                                                                                             |
| `bun run fingerprints:capture <provider>` · `fingerprints:import-har` · `fingerprints:diff` · `fingerprints:bless` · `fingerprints:list` | the fingerprint capture harness (`src-tauri/crates/bluey-fingerprints`): point the official CLI at a local proxy, import a HAR, diff a capture against the documented fingerprint and the blessed golden, bless it — `docs/PROVIDER_ACCOUNTS.md › Re-capture runbook` |
| `bun run check:rust`                   | fmt + tests + clippy; add `--darwin` to type-check the app crate for macOS                                                                                                                            |
| `bun run build:helper` / `build:agent` | native helper / research agent sidecars                                                                                                                                                               |
| `bun run tauri icon`                   | regenerate `src-tauri/icons/` (`.icns`, `.ico`, PNG sizes) from the 1024×1024 master `app-icon.png`; the menu-bar glyphs in `src-tauri/icons/tray/` are hand-made (see `docs/DESIGN.md`)              |
| `bun run tauri:build`                  | production `.app` + `.dmg` (see `scripts/release.sh` for signing)                                                                                                                                     |

## Providers and the `.env` import

On boot the Rust backend imports provider settings from the environment (`app::env_import`,
planned by `bluey_core::presets::plan_env_import`, ADR 0007):

- Keys (`GEMINI_API_KEY` / `GOOGLE_API_KEY`, `AZURE_FOUNDRY_API_KEY`, `ANTHROPIC_API_KEY`,
  `OPENAI_API_KEY`) are copied into the macOS Keychain **only when it has no entry for that
  provider**; `BLUEY_ENV_OVERRIDES_KEYCHAIN=1` replaces existing entries. The log says
  `imported api key for provider gemini` and nothing else — delete the key from `.env` afterwards
  if you like.
- `BLUEY_AI_PROVIDER` (`gemini` default | `azure-foundry` | `anthropic` | `openai`) nominates the
  default provider; its recommended models fill every role that is still unassigned and
  `BLUEY_MODEL_*` override single roles. Settings → AI → *Default AI provider* does the same with
  one click, and *Use recommended models* re-applies a provider's presets. Only a **changed**
  `.env` value re-applies presets over your edits and re-nominates the default: the import
  remembers what the environment nominated last time (settings table,
  `env_import:bootstrap_provider`), so a default provider you switched to in Settings — ChatGPT,
  say — is still the default after a relaunch.
- `BLUEY_EMBEDDING_DIMENSIONS` (768 / 1536 / 3072), `BLUEY_TRANSCRIPTION_PROVIDER`
  (`gemini_live` default | `apple` | `cloud_realtime`) and `RESEARCH_BACKEND` (`gemini` default |
  `claude`) set the matching settings on the first launch or when `BLUEY_AI_PROVIDER` changes;
  after that, Settings wins (the import never reverts your edits).
- The research sidecar ships as the **lite** binary (Gemini) unless `RESEARCH_BACKEND=claude`
  (or `BLUEY_AGENT_VARIANT=full`) at build time, which embeds the Claude CLI.

## Subscription accounts (ADR 0009)

Settings → AI → **Accounts** signs in with the owner's ChatGPT, Claude or Google AI subscription
(unofficial; see `docs/PROVIDER_ACCOUNTS.md`). Two switches: the Cargo feature
`subscription-accounts` (in `default`; build with `--no-default-features --features dev-tools` to
compile the layer out — every account then reports `unavailable`) and the runtime setting
`experimental.subscriptionAccounts` (the switch in the Accounts section; off hides the cards and
refuses new sign-ins). The `mock` transport (`bun run dev`) ships fake accounts that connect on a
timer with fixture catalogs; the real provider profiles land in PR 3a–3c. The providers' request
fingerprints are data in `bluey_protocols::fingerprints` (`VERSION` / `CAPTURED_ON`, rules, the
documented captures under `tests/fixtures/fingerprints/<provider>/documented/`); after every
official-client release, `bun run fingerprints:capture <provider>` + `bun run fingerprints:diff
<provider>` tells you which doc column and which constant to update.

## Working without macOS

The platform-independent Rust crates and the whole TypeScript codebase build and test on Linux.
`scripts/check-rust.sh --darwin` type-checks the macOS app crate from Linux using a stub C
compiler for `cc`-based build scripts (`cargo check` never links). The Swift helper and the
final bundle require macOS.

## Project conventions

- **Contracts first.** `src/lib/types` ⇄ `crates/bluey-core/src/types` are mirrored;
  `src/lib/tauri/commands.ts` and `events.ts` are the command/event surface. Change both sides.
- No `invoke()` outside `src/lib/tauri`; use `bluey.*`. No SQL outside `bluey-storage`.
  No provider HTTP outside `src-tauri/src/ai`. No prompts outside `src/ai/prompts`.
- State comes from the Rust state machine (`app.state` event); UI never invents `isX` flags.
- Errors are `BlueyError` everywhere; UI maps `recovery` to actions.
- Small modules; feature folders; tests next to logic (`tests/unit`, `tests/integration`,
  `tests/ui`, Rust `#[cfg(test)]`).

## Developer Mode

Settings → General → _Developer mode_ (or `?dev=1` in the browser) enables:

- Simulate: question, coding problem, transcript, screen capture (fixture), permission error,
  AI latency, AI failure (`dev_simulate`).
- Metrics overlay: capture / OCR / transcript / context assembly / model latency, TTFT, tokens.
- The `mock` AI provider (deterministic streamed answers) and the mock transcription provider.
  Everything under `src/lib/tauri/mock/` and the Rust `dev` module is isolated from production
  paths.

## Debugging

- Rust logs: `RUST_LOG=bluey=debug bun run tauri:dev` or Settings → Advanced → Log level.
  Logs are JSON lines in `~/Library/Logs/Bluey/`.
- Helper: run it directly and pipe requests — see `tests/native/README.md`.
- Agent sidecar: `BLUEY_AGENT_MOCK=1` runs the protocol end-to-end without network — see
  `sidecars/agent/README.md`.
- WebView devtools: right-click → Inspect in debug builds.

## Release

`scripts/release.sh` retains local build-only `.app`/`.dmg` output and the lite Gemini/full
Claude sidecar choice. Release builds require **Bun 1.4.2** and frozen root/sidecar installs.
No-credential builds are developer-only and can never become publication-eligible artifacts.
The existing helpers build both sidecar architectures; Tauri selects the requested target.

Every build also writes the in-app updater bundle (`Bluey.app.tar.gz` + `.sig`, see
[Updates](UPDATES.md)), so `scripts/release.sh` and `bun run tauri build` need
`TAURI_SIGNING_PRIVATE_KEY` (+ `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) in the environment — the
owner's key from its backup, or a throwaway pair from `bun run tauri signer generate -w /tmp/dev.key`
for builds that will never feed real installs. `bun run tauri dev` does not sign anything.

The active `.github/workflows/release.yml` publishes only from existing version tags, after
complete signing/notarization/native validation of both macOS DMGs. Manual dispatch defaults
to **build-only**; publication must be explicitly requested with an existing tag. See
[Releasing](RELEASING.md) for owner credentials, environment/tag protection, manual instructions,
manifest schema, failure/re-run semantics and the native checks that Linux cannot perform.
Workflow install copies in `docs/ci/workflows/` are kept in sync with the release workflow.
