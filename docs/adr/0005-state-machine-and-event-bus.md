# ADR 0005 — One state machine, one event bus, generations for staleness

**Status:** accepted · **Date:** 2026-09-07

## Context
The spec forbids scattering `isListening/isThinking/isLoading` flags and demands explicit
transitions, cancellation and stale-response protection.

## Decision
* `bluey_core::state::AppStateMachine` is the single source of truth (`AppStatus` with the
  primary `state` and one orthogonal `audioActive` region). Every change goes through
  `transition(AppEvent)`; invalid transitions are errors, not silent no-ops. The frontend only
  mirrors `app.state` events.
* `bluey_core::events::BlueyEvent` enumerates every event; Rust publishes them on a tokio
  broadcast bus and forwards each to the WebView as the Tauri event `bluey:` + the dotted name
  with `.` replaced by `/` (`app.state` → `bluey:app/state`) — Tauri v2 only accepts
  `[A-Za-z0-9-/:_]` in event names, and `emit` fails silently for the WebView otherwise. Both
  sides derive the wire name from the same rule (`tauri_event_name_for` / `tauriEventName`) and
  tests assert every name stays inside that alphabet. The frontend `eventBus` is the only
  subscriber API.
* Every AI request carries `requestId`, `sessionId`, `generation`. The TS `GenerationGate`
  increments a per-scope generation on each new ask, cancels the previous request
  (`ai_cancel` → `CancellationToken` in Rust), and drops chunks whose generation is stale, so a
  slow older response can never overwrite a newer one.

## Consequences
* UI states are derivable and testable; the HUD renders from `AppStatus.state` alone.
* Adding an event means adding a variant + a TS payload type — the round-trip is covered by a
  test comparing `EVENT_NAMES` with the Rust `name()` list.
