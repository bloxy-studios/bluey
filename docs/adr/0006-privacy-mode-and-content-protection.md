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

## Addendum 2026-09-28 — macOS 15+ and native menus (SEC-004, SEC-012)
The decision above overstated the coverage. `NSWindow.sharingType = .none` is honoured by
legacy window capture (CGWindowList and apps built on it), but ScreenCaptureKit on macOS 15
and later may still capture a protected window, and most current screen sharing and recording
(Zoom, Meet, Teams, QuickTime, OBS) uses ScreenCaptureKit. NSMenu popups (the HUD's native
menus) are separate windows that are never protected. This was reported by others and not
reproduced here; until a runtime self-test exists, Bluey reports the weaker claim:

* `CaptureProtection.partial` is true whenever protection is on and macOS is 15 or later (or
  its version is unknown). The note then reads: hidden from apps that honour macOS window
  protection (legacy capture); modern ScreenCaptureKit screen sharing on macOS 15+ may still
  show Bluey, and its menus are never hidden.
* Protection is still applied (`set_content_protected`): it helps with legacy capture.
* It is applied before the HUD is first shown at boot, never a few seconds later (SEC-012).
* The tray's *Toggle Privacy Mode* patches `settings.privacy.displayMode`, the single source
  of truth, so the tray, the HUD and the Privacy Center always agree.

Bluey's own captures still exclude its windows through the ScreenCaptureKit content filter.
