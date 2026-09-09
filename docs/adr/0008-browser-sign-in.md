# ADR 0008 — Sign in through the system browser (Clerk as OAuth/OIDC provider)

**Status:** accepted · **Date:** 2026-09-09 · supersedes ADR 0003

## Context
ADR 0003 ran clerk-js inside the Tauri WebView in Clerk's "native" mode (client JWT in an
`Authorization` header, `standardBrowser: false`). WKWebView adds an `Origin` header
(`tauri://localhost`, `http://localhost:1420` in dev) to every request, and Clerk's Frontend API
rejects `Origin` + `Authorization` together (`origin_authorization_headers_conflict`). The
documented fallback — proxying every Frontend API call through Rust — keeps a browser SDK alive
inside a WebView it was never meant for, and keeps the Clerk client token in the renderer.

## Decision
Bluey signs users in the way desktop tools do: **in the default browser**, with the result handed
back to the app.

* Clerk is used as an **OAuth 2.0 / OpenID Connect provider**. Bluey is a *public* OAuth
  application of the Clerk instance (PKCE `S256`, no client secret) — created once in the Clerk
  Dashboard with the redirect URIs `bluey://auth/callback` and `http://127.0.0.1/callback` and
  the scopes `openid profile email offline_access`.
* **Rust owns the flow** (`src-tauri/src/auth`): `auth_begin_sign_in` generates PKCE + `state` +
  `nonce`, opens `{issuer}/oauth/authorize` with the system opener and waits for the redirect —
  the `bluey://auth/callback` deep link (installed builds; `tauri-plugin-deep-link`) or a one-shot
  loopback listener on `127.0.0.1:<random port>` (development builds, which macOS does not
  register for deep links; `BLUEY_AUTH_REDIRECT` overrides). It validates `state`, exchanges the
  code at `/oauth/token`, checks the ID token's issuer/audience/nonce/expiry, loads
  `/oauth/userinfo`, stores the tokens in the Keychain (`auth:clerk:oauth_tokens`) and the
  identity in SQLite, and brings the originating window to the front.
* **The WebView never talks to Clerk.** `@clerk/*` is gone from the bundle; the frontend sees only
  `AuthStatus` (`auth_get_status`, `auth.changed`) and drives the flow with
  `auth_begin_sign_in` / `auth_cancel_sign_in` / `auth_clear_session`. The CSP no longer allows
  Clerk hosts for scripts, connections or frames (avatars from `img.clerk.com` stay allowed).
* Profile and security settings live on Clerk's hosted **Account Portal**
  (`auth_open_account_portal`, derived from the Frontend API host or
  `BLUEY_CLERK_ACCOUNT_PORTAL_URL`); Bluey shows the identity it holds and offers sign-out
  (best-effort token revocation).
* At boot the stored session is validated/refreshed in the background (`refresh_token` grant);
  a session Clerk rejects signs the user out, being offline keeps the cached identity.

## Consequences
* One-time setup step per Clerk instance: create the public OAuth application and put its client
  id in `BLUEY_CLERK_OAUTH_CLIENT_ID` (`docs/DEVELOPMENT.md`). Without it the app runs without
  an account, as before.
* The pure half (URLs, PKCE, redirect parsing, token/ID-token/userinfo decoding, Account Portal
  derivation, loopback request parsing) is in `bluey_protocols::clerk` and unit-tested; the I/O
  half is thin.
* No Clerk secret key is ever used or shipped; only the publishable key and the public client id.
* Sign-in from within the HUD still routes to the onboarding window (the HUD panel cannot host the
  card); the browser round-trip needs the app to be reachable for the deep link — a build under
  `tauri dev` uses the loopback automatically.
