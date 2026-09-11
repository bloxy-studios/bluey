# ADR 0009 — Provider accounts: the owner's AI subscriptions as credential sources, fingerprints as dated data

**Status:** accepted · **Date:** 2026-09-11 · companion: `docs/PROVIDER_ACCOUNTS.md`,
`docs/reference/provider-accounts-fast-path-brief.md` §2–§4c

## Context
Every AI role in Bluey is served by a provider adapter that authenticates with an API key from the
Keychain (ADR 0001, ADR 0007). The owner already pays for ChatGPT, Claude and Google AI
subscriptions. Their official clients — Codex CLI, Claude Code, the Antigravity IDE — draw on those
plans through OAuth flows against vendor backends that are not offered as APIs: the Codex backend
(`chatgpt.com/backend-api/codex`), the Messages API in the Claude Code wire format, and Google's
Cloud Code internal API (`cloudcode-pa.googleapis.com/v1internal`). Community tools — CLIProxyAPI,
vibeproxy, T3 Code, OpenClaw, the opencode auth plugins — use exactly those flows, and they keep
working only for as long as a request looks like the official client. Vendors act on requests they
do not recognise: Anthropic has validated the Claude Code shape since 2026-01 and since 2026-04
bills unrecognised OAuth requests to paid "extra usage" instead of the plan; Google's Antigravity
terms (read 2026-09-11) state that *"using third party software, tools, or services to access the
Service (e.g. using OpenClaw with Antigravity OAuth) is a breach of this Agreement"* that *"may be
grounds for suspension or termination of your Antigravity and/or Gemini CLI accounts"*, and
account-level 403 suspensions have been documented since 2026-02; OpenAI publishes no rule either
way but validates the Codex request shape and can change it without notice.

Bluey is a personal, single-user tool (README: "no billing, subscriptions or teams" of its own) —
one person, their own accounts, their own Mac. The owner has decided that terms-of-service
considerations do not limit feature scope; they enter this design only as engineering inputs.

## Decision
1. **All three subscriptions become credential sources next to API keys.** API keys stay the
   primary path and are always the fallback target.

   | Credential source | Built | Engineering notes |
   |---|---|---|
   | API keys (Gemini, Foundry/Azure, Anthropic, OpenAI-compatible) | today | unchanged; fallback when an account is `NeedsReauth`, `Unavailable` or rate-limited |
   | ChatGPT Free/Plus/Pro — Codex OAuth | PR 3a | PKCE on the fixed loopback port 1455 → device-code fallback; Responses dialect to the Codex backend; catalog from the Codex models endpoint; plan from the ID token |
   | Claude Pro/Max — claude.ai OAuth, Claude Code wire format | PR 3b | existing Anthropic Messages codec + a `ClaudeCodeShaper` pinned to a captured Claude Code version; extra-usage guard; optional Console sign-in → API key |
   | Google AI Pro/Ultra — Antigravity OAuth, Cloud Code `v1internal` | PR 3c | Google OAuth with the Antigravity client (five scopes, fixed loopback port 51121); `loadCodeAssist` for project + tier; the Code Assist wrapper on one host per account (the maintained reference removed cross-host fallback in 2026-08), the thin header set the official client actually sends, one connection per account; catalog from `fetchAvailableModels`; several Google accounts rotated only on quota exhaustion — and all of them stopped together on a Terms-of-Service 403 |

2. **One Accounts abstraction, not three special cases.** `ProviderAuthMethod { ApiKey,
   OAuthSubscription }`; `ProviderAccount` with `AccountStatus { Disconnected, Connecting,
   Connected, NeedsReauth, RateLimited { until, window }, Unavailable { reason } }`;
   `AccountsManager` in Rust owns connect / import / cancel / disconnect / restore / refresh and
   resolves a `CredentialSource` per request. The provider-agnostic OAuth machinery — PKCE,
   loopback listener (random or *required* fixed port), manual code paste, device code, token set,
   single-flight refresh — is extracted from the Clerk sign-in (ADR 0008) into `src-tauri/src/oauth`
   and `bluey_protocols::oauth`, with Clerk as its first consumer and no behaviour change. Each
   provider contributes an OAuth profile, a `RequestShaper`, a catalog fetcher and an error mapper;
   the router treats a non-`Connected` account as keyless and skips a `RateLimited` one until
   `until`.
3. **Fingerprints are data with a date.** Each provider's request shape lives in one module —
   `bluey_protocols::fingerprints::{codex, claude_code, antigravity}` — with `VERSION`,
   `CAPTURED_ON`, a scrubbed golden capture under `tests/fixtures/fingerprints/`, and drift
   detection driven by the provider's own signals (error strings, status codes). A dev capture
   proxy and `bun run fingerprints:diff` compare the shipped shaper against a fresh capture of the
   official client. Any PR that touches a header or system block for one of these providers
   updates the fixture and the date.
4. **Never spend the owner's money silently.** A response saying the request is billed to extra
   usage or pay-as-you-go instead of the plan is a *stop* signal: the request halts, the account
   flips to `Unavailable { ExtraUsageBilling | FingerprintDrift }`, the role falls back and the user
   is told once. A drifted fingerprint is never retried automatically — a human re-captures and
   bumps `VERSION`.
5. **The fallback order is explicit and visible:** subscription account → API-key provider for the
   same role → the router's existing chain. A rate-limited account shows its window and reset time
   and is skipped until then; nothing waits in a loop inside an interactive HUD.
6. **The secret boundary does not move; the allow-list narrows.** Tokens live under
   `account:<account_id>:oauth_tokens`, read and written only by `AccountsManager`; the WebView
   sees `ProviderAccount` (status, plan, e-mail, project) and nothing else; `secrets_set` /
   `secrets_delete` accept only `provider:<id>:api_key`; redaction gains the new token shapes; no
   subscription token reaches a child process — the research sidecar keeps using API keys.
7. **Consent once, flags always.** One consent dialog per provider before the browser opens
   (what is sent, whose plan limits are used, that the integration is unofficial and may stop
   working, what Bluey does when it does). `settings.experimental.subscriptionAccounts` (default
   on — it is the point of the feature; kept so a broken provider can be switched off without a
   rebuild) plus the Cargo feature `subscription-accounts`; a startup self-check per account flips
   it to `Unavailable` with a readable reason when its endpoint is gone.
8. **Catalogs are discovered, never hard-coded.** Codex: the models endpoint; Claude: `/v1/models`
   if it accepts OAuth, else a `Curated { version }` list from the capture; Antigravity:
   `fetchAvailableModels`, with a probe as fallback. Presets apply after the first catalog fetch
   and never assign a model the catalog did not return.
9. **Importing an existing sign-in is read-only.** Tokens from Claude Code, Codex CLI or the
   Antigravity app are copied into Bluey's own Keychain entry; their stores are never written.

## Consequences
* New provider kinds `chatgpt_codex`, `claude_subscription`, `antigravity_google` with reserved
  ids `chatgpt`, `claude`, `antigravity`; commands `accounts_*`; events `accounts.changed`,
  `accounts.catalog`; `MockTransport` fake accounts with fixture catalogs; an Accounts section in
  Settings → AI, an onboarding branch, Reconnect / Use-API-key pills in the HUD.
* Maintenance is a re-capture, not a rewrite. Every official-client release may move a
  fingerprint; the capture harness, the golden fixture and the `VERSION` bump ship in one PR.
* Google is the provider whose vendor terms name the practice explicitly. The consent dialog
  quotes them verbatim, and a Terms-of-Service 403 on one Google account stops every connected
  Google account at once (no rotation) — the owner decides whether to reconnect.
* Every dated vendor fact — endpoints, headers, scopes, claim names, model ids, client versions —
  lives in `docs/PROVIDER_ACCOUNTS.md` with its verification date and source, and is re-checked on
  the day it is implemented. Code carries constants, not undated beliefs.
* The app crate gets its first `#[tokio::test]`s (OAuth engine, `AccountsManager`, adapters
  against a stub HTTP server); `docs/SECURITY.md` gains the invariants above; `docs/TESTING.md`
  gains the QA lines. PR order: brief §6 (PR 1 hardening + OAuth engine, PR 2 accounts layer,
  PR 2b capture harness, PR 3a/3b/3c providers).
* Sources for the wire facts, checked 2026-09-11: `router-for-me/CLIProxyAPI` (primary,
  maintained), `automazeio/vibeproxy`, `EvanZhouDev/openai-oauth`, the Codex CLI source,
  `shahidshabbir-se/opencode-anthropic-oauth`, `kristianvast/hermes-claude-auth`, pi-ai,
  `NoeFabris/opencode-antigravity-auth` (archived), `firdyfirdy/antigravity-auth`, and the
  `google-gemini/gemini-cli` Code Assist client — itemised in `docs/PROVIDER_ACCOUNTS.md`.

## Explicitly out of scope
Hosted proxies, relaying or sharing tokens, anything multi-tenant, subscription tokens in the
research sidecar or any other child process, writing to other clients' credential stores, and
adding product gates the owner did not ask for.
