# ADR 0006 — Privacy mode uses platform content protection only

**Status:** accepted · **Date:** 2026-09-07

## Context
Users may not want the Bluey panel to appear in screen shares or recordings. The spec is
explicit: use supported capture-protection mechanisms, never anti-cheat/monitoring evasion, and
never claim universal invisibility.

## Decision
"Privacy" display mode calls Tauri's `WebviewWindow::set_content_protected(true)`, which sets
`NSWindow.sharingType = .none` on macOS. This excludes the panel from most screen-capture APIs
(ScreenCaptureKit, CGWindowList-based capture used by Zoom/Meet/Teams/QuickTime) but macOS does
**not** guarantee exclusion from every path (e.g. hardware capture cards, some virtual display
drivers). The `CaptureProtection` object reports `supported`, `enabled` and an honest `note`
shown in the Privacy Center and in the HUD tooltip ("Content-protected" vs "Detectable").

Bluey excludes its own windows from its *own* captures via the ScreenCaptureKit content filter
so screenshots sent to models never include the HUD.

## Explicitly not implemented
Cursor deception, fake input, process hiding, or any mechanism meant to defeat third-party
monitoring or proctoring systems.
