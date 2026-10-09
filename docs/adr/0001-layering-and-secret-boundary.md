# ADR 0001 — Layering and the secret boundary

**Status:** accepted · **Date:** 2026-09-07

## Context
Bluey combines native capture, audio, storage, multiple AI providers and a WebView UI. The
spec requires that the renderer never holds provider secrets, that native work does not run in
JavaScript, and that capture / preprocessing / context / intelligence / generation /
presentation stay decoupled.

## Decision
Four processes, five layers:

| Layer | Where | Owns |
|---|---|---|
| Presentation | WebView (React) | HUD, settings, onboarding, stores mirroring backend state |
| Intelligence | WebView (`src/ai`, `src/context`, `src/transcript`, `src/modes`) | prompt building, context fusion, relevance, token budget, response optimisation, question detection, research routing |
| Orchestration | Rust Tauri app (`src-tauri/src`) | commands, state machine, event bus, sessions, model routing execution, provider HTTP with credentials, sidecar lifecycle, storage, secrets (macOS Keychain), NSPanel, shortcuts, tray |
| Native capability | Swift helper sidecar (`src-tauri/swift/BlueyHelper`) | ScreenCaptureKit, Vision OCR, Accessibility, AVAudioEngine, SCStream audio, Apple Speech |
| Agentic research | Bun sidecar (`sidecars/agent`) | Claude Agent SDK loop with scoped Exa / Firecrawl / document tools |

The **secret boundary** is the Rust process: API keys and the Clerk client token live in the
Keychain (`keyring`), are injected into provider requests or sidecar environments by Rust, and
are never returned to the WebView (`secrets_set/has/delete` only). Prompts are built in TS
(they contain no secrets) and shipped to Rust as an `AIRequest`; Rust selects the model and
streams `AIChunk`s back over a Tauri `Channel`.

## Consequences
* Platform-independent Rust logic lives in `bluey-core` / `bluey-storage` (testable on any OS).
* The Tauri app crate is macOS-only and is type-checked cross-platform.
* Adding a provider = one Rust adapter; adding a mode = data; adding a native capability =
  one helper method + one command.

## Addendum 2026-10-09 — research backend and secret storage (DOC-011, ADR 0007, ADR 0011)
* The research sidecar's default backend is Gemini function calling (ADR 0007); the Claude
  Agent SDK loop is the opt-in `claude` backend. Both get the same scoped Exa / Firecrawl /
  document tools.
* Secrets no longer go through the `keyring` crate: `secrets::keychain::KeychainBackend` calls
  `security-framework` directly behind the `SecretsStore` cache, and debug builds use their own
  service, `com.codewithabdul.bluey.dev` (ADR 0011). The Clerk session comes from the browser
  sign-in Rust runs (ADR 0008).
* The boundary itself is unchanged: values never reach the WebView. Besides
  `secrets_set/has/delete`, the WebView sees only states — `secrets_state` (present / locked /
  absent), `secrets_health` (Settings → Privacy → *Saved credentials*, names and states) — and
  can ask for `secrets_allow_access`, the one deliberate interactive read, which returns no value.
