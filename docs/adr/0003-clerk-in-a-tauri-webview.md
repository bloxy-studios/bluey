# ADR 0003 — Clerk authentication inside a Tauri WebView

**Status:** accepted (with documented fallbacks) · **Date:** 2026-09-07

## Context
Authentication is required through Clerk. The production WebView origin is `tauri://localhost`
(dev: `http://localhost:1420`). Clerk's browser flow relies on the `__client` cookie on the
Frontend API domain, which does not work from a custom-scheme origin, and Clerk states it does
not officially support Tauri/Electron.

## Decision
Use `@clerk/react` 6 with `standardBrowser={false}` (Clerk's documented mode for native
platforms) and bundle `@clerk/ui` so no CDN script is loaded in the sandboxed WebView. Persist
the client JWT (`__clerk_client_jwt`) through Rust (`auth_store_session` /
`auth_load_client_token` → Keychain) using the same request/response hooks the official Expo
SDK uses (`__internal_onBeforeRequest` adds `authorization` + `_is_native=1` and omits
credentials; `__internal_onAfterResponse` saves the returned `authorization` header). On
relaunch the JWT is replayed during `Clerk.load()` and the last active session is restored.

Requirements on the Clerk instance: **Native applications** enabled; `allowed_origins` includes
`tauri://localhost` (and `http://localhost:1420` for development).

## Fallbacks
1. If the Frontend API rejects `Origin` + `Authorization` together, route Clerk FAPI calls
   through Rust with `auth_fapi_fetch` (a strict https-only proxy limited to the Clerk domain).
2. Social sign-in that needs a browser redirect opens the system browser; Bluey registers the
   `bluey://` scheme (deep-link plugin) for the return trip.

## Consequences
* The hooks are internal Clerk API — versions are pinned and covered by an integration test.
* No Clerk secret key is ever used or shipped; only the publishable key is in the frontend.
