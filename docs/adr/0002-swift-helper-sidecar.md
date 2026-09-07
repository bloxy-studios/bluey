# ADR 0002 — Native capabilities in a Swift helper sidecar

**Status:** accepted · **Date:** 2026-09-07

## Context
ScreenCaptureKit, Vision, Accessibility, AVFoundation/CoreAudio and Speech are Swift/ObjC
frameworks. Binding them from Rust via `objc2` is possible but slow to develop, hard to keep
current with macOS releases, and drags Apple SDK specifics into the Rust core.

## Decision
A separate SwiftPM executable (`bluey-helper`) bundled as a Tauri `externalBin`, spoken to over
stdio JSON Lines (`docs/HELPER_PROTOCOL.md`). Rust owns its lifecycle (spawn after auth, restart
with backoff, timeouts per method) and translates helper errors into `BlueyError`.

Only two permission checks stay in Rust because they are one-line C calls and must be attributed
to the main process: `CGPreflightScreenCaptureAccess` (core-graphics) and `AXIsProcessTrusted`
(accessibility-sys). Microphone and speech authorization requests go through the helper; TCC
attributes child processes to the responsible app bundle, so prompts show "Bluey".

## Alternatives considered
* **objc2 bindings in Rust** — rejected for velocity and maintenance.
* **Node/Electron-style native modules** — rejected; the spec forbids native work in JS.
* **XPC service** — better isolation but much heavier packaging; may be revisited.

## Consequences
* Two builds per architecture (arm64, x86_64) via `scripts/build-helper.sh`; end users never
  need Swift.
* Images cross the boundary as temp files (fast, no base64 blow-up); audio as small base64
  chunks; on-device transcripts as events.
* The helper cannot be compiled in a Linux CI container; macOS CI is required for it.
