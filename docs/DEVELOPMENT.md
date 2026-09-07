# Development

## Prerequisites (macOS)
* macOS 14+, Xcode Command Line Tools (`xcode-select --install`) — provides Swift 5.9+.
* [Bun](https://bun.sh) ≥ 1.2, [Rust](https://rustup.rs) stable (≥ 1.85) with
  `rustup target add aarch64-apple-darwin x86_64-apple-darwin`.
* A Clerk application (publishable key) with **Native applications** enabled and
  `tauri://localhost` + `http://localhost:1420` in allowed origins.
* Optional: Foundry/Azure OpenAI or Anthropic API key, Exa / Firecrawl keys.

## Setup
```bash
bun install
cp .env.example .env            # fill in VITE_CLERK_PUBLISHABLE_KEY at minimum
bun run build:helpers           # Swift helper + Bun agent sidecars → src-tauri/binaries/
bun run tauri:dev               # Vite + Rust + the app
```

Other commands:

| Command | What |
|---|---|
| `bun run dev` | Vite only — the UI in a browser with the **MockTransport** (Developer Mode backend): `http://localhost:1420/?window=main`, `?window=settings`, `?window=onboarding`, add `&dev=1` for the dev overlay |
| `bun run typecheck` | `tsc --noEmit` for the app and the agent sidecar |
| `bun run lint` / `bun run format` | ESLint (strict TS, hooks) / Prettier |
| `bun run test` | Vitest (unit, integration, UI) |
| `bun run test:rust` | `cargo test` for `bluey-core` and `bluey-storage` |
| `bun run check:rust` | fmt + tests + clippy; add `--darwin` to type-check the app crate for macOS |
| `bun run build:helper` / `build:agent` | native helper / research agent sidecars |
| `bun run tauri:build` | production `.app` + `.dmg` (see `scripts/release.sh` for signing) |

## Working without macOS
The platform-independent Rust crates and the whole TypeScript codebase build and test on Linux.
`scripts/check-rust.sh --darwin` type-checks the macOS app crate from Linux using a stub C
compiler for `cc`-based build scripts (`cargo check` never links). The Swift helper and the
final bundle require macOS.

## Project conventions
* **Contracts first.** `src/lib/types` ⇄ `crates/bluey-core/src/types` are mirrored;
  `src/lib/tauri/commands.ts` and `events.ts` are the command/event surface. Change both sides.
* No `invoke()` outside `src/lib/tauri`; use `bluey.*`. No SQL outside `bluey-storage`.
  No provider HTTP outside `src-tauri/src/ai`. No prompts outside `src/ai/prompts`.
* State comes from the Rust state machine (`app.state` event); UI never invents `isX` flags.
* Errors are `BlueyError` everywhere; UI maps `recovery` to actions.
* Small modules; feature folders; tests next to logic (`tests/unit`, `tests/integration`,
  `tests/ui`, Rust `#[cfg(test)]`).

## Developer Mode
Settings → General → *Developer mode* (or `?dev=1` in the browser) enables:
* Simulate: question, coding problem, transcript, screen capture (fixture), permission error,
  AI latency, AI failure (`dev_simulate`).
* Metrics overlay: capture / OCR / transcript / context assembly / model latency, TTFT, tokens.
* The `mock` AI provider (deterministic streamed answers) and the mock transcription provider.
Everything under `src/lib/tauri/mock/` and the Rust `dev` module is isolated from production
paths.

## Debugging
* Rust logs: `RUST_LOG=bluey=debug bun run tauri:dev` or Settings → Advanced → Log level.
  Logs are JSON lines in `~/Library/Logs/Bluey/`.
* Helper: run it directly and pipe requests — see `tests/native/README.md`.
* Agent sidecar: `BLUEY_AGENT_MOCK=1` runs the protocol end-to-end without network — see
  `sidecars/agent/README.md`.
* WebView devtools: right-click → Inspect in debug builds.

## Release
`scripts/release.sh` — installs, checks, builds both sidecars for the target, runs
`tauri build`, signs and notarizes when `APPLE_SIGNING_IDENTITY`/`APPLE_ID`/`APPLE_PASSWORD`/
`APPLE_TEAM_ID` are present, and prints the `.app`/`.dmg` paths.
