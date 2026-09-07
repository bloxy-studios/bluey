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
