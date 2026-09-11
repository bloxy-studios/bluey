# Security & Privacy Model

## Principles
1. **Nothing happens silently.** Capture and listening only start on explicit user action
   (shortcut, HUD button, menu bar) and are always visible (HUD pill, menu bar item).
2. **Secrets never reach the renderer.** API keys and the sign-in tokens are stored in the
   macOS Keychain by the Rust process (`keyring`). The WebView can only `set`, `has`, `delete`
   the API keys listed in `SECRET_KEYS` (`src/lib/tauri/commands.ts`: provider, Exa, Firecrawl
   and agent keys); the `secrets_*` commands reject every other key (`auth:*`, `account:*`)
   before the store is touched, and the WebView never sees a token at all.
3. **Minimal retention by default.** Raw audio is never persisted; screenshots are off by
   default; transcripts and session history can be disabled; deletion really deletes.
4. **Model output is untrusted data.** It is rendered as text/markdown, never executed, and the
   system prompt instructs models that screen/transcript content is data, not instructions.
5. **Least privilege everywhere.** Per-window Tauri capabilities, scoped agent tools, no shell
   access from the frontend, no arbitrary native command execution.

## Secret handling
| Secret | Where | How it is used |
|---|---|---|
| Provider API keys (Gemini, Foundry/Azure, Anthropic, OpenAI-compatible) | Keychain `provider:<id>:api_key` | Injected into HTTPS headers by Rust provider adapters (`x-goog-api-key`, `api-key`, `x-api-key`, `Authorization`) — never in URL queries, except the Gemini Live WebSocket URL, which is redacted from logs |
| Gemini key for the research sidecar | Keychain `provider:gemini:api_key` (same entry) | `GEMINI_API_KEY` env of the sidecar process only, when `RESEARCH_BACKEND=gemini` |
| Exa / Firecrawl keys | Keychain | Rust research clients; env-injected into the agent sidecar per job |
| Anthropic key for the agent | Keychain (or `ANTHROPIC_API_KEY` in Bluey's `.env`) | `ANTHROPIC_API_KEY` env of the sidecar process only, when `RESEARCH_BACKEND=claude` |
| Sign-in tokens (OAuth access / refresh / ID token) | Keychain `auth:clerk:oauth_tokens` | Rust only (ADR 0008): browser sign-in via Clerk's OAuth/OIDC endpoints; validated/refreshed at boot; revoked on sign-out |
| AI subscription tokens (ChatGPT / Claude / Google — ADR 0009, from the Provider Accounts PRs) | Keychain `account:<account_id>:oauth_tokens` | Rust only (`AccountsManager`): refreshed under a single-flight lock, injected into provider requests by the adapter, never returned to the WebView, never passed to a sidecar |
| Clerk publishable key + public OAuth client id | `VITE_CLERK_PUBLISHABLE_KEY`, `BLUEY_CLERK_OAUTH_CLIENT_ID` (public by design): the environment / `.env.local` / `.env` at startup, else the values `src-tauri/build.rs` compiled in from the same files (an explicit allowlist of public identifiers — never API keys) | Rust derives the issuer; the WebView never talks to Clerk |

Sidecars are spawned with a **cleared environment**: the helper and the research agent receive
only `PATH`/`HOME`/`TMPDIR`/`USER`/`LANG` plus the variables Rust passes explicitly
(`AgentManager::job_env` — exactly one research backend's credentials, the Exa/Firecrawl keys and
documented `BLUEY_*` knobs). Keys that `.env` loads into Bluey's own process therefore never reach
a child process that has no business with them. Log lines are redacted for `sk-…`, `fc-…`,
`AIza…`, `Bearer …`, `api-key` values and `key=` URL queries.

Keys entered in Settings are written straight to the Keychain and the UI only shows
"Key saved". `.env` values are imported into the Keychain on first run and can be removed from
disk afterwards. Nothing secret is written to SQLite or logs; the logger redacts common key
patterns (`sk-…`, `fc-…`, bearer tokens) defensively.

## Provider accounts — subscription sign-in (ADR 0009)

The Provider Accounts PRs (`docs/PROVIDER_ACCOUNTS.md` lists which PR enforces what) add a second
credential source next to API keys: the owner's own ChatGPT, Claude and Google AI subscriptions,
signed in through the vendors' OAuth flows. These invariants hold for every one of them:

* **Tokens are Rust-only.** They live under `account:<account_id>:oauth_tokens`, are read and
  written only by `AccountsManager` (`src-tauri/src/accounts`), and never reach the WebView, the
  logs, SQLite or a child process. The WebView sees `ProviderAccount` — status, plan, e-mail,
  project id — and nothing else; the account list and the model catalogs persisted in the
  settings table carry no credential. A test asserts that the research sidecar's environment
  never names OAuth or account material.
* **The WebView's secret allow-list narrows, it does not widen.** `secrets_set` / `secrets_has` /
  `secrets_delete` accept only the `SECRET_KEYS` of `commands.ts` — `provider:<id>:api_key` and
  the research / agent API keys; `auth:*` and `account:*` are rejected at the command layer
  (`secrets::validate_webview_key`), and `SecretsStore::validate_key` stays as the storage-level
  allow-list.
* **Redaction grows with the tokens**: `chatgpt-account-id`, `sk-ant-oat…`, `sk-ant-ort…`,
  `ya29.…` and `1//…` join the log patterns above.
* **Loopback listeners** bind `127.0.0.1` only, accept a single request of ≤ 8 KB with a 5 s read
  timeout, and check `state` before anything else. Manual code paste (`code#state`) is validated
  the same way.
* **No silent spending.** A response saying the request is billed to extra usage or pay-as-you-go
  instead of the plan is a *stop* signal: the request halts, the account flips to
  `Unavailable{fingerprint_drift | extra_usage_billing}`, Bluey falls back to the API-key provider
  and tells the user. A drifted fingerprint is never retried automatically.
* **Captures are scrubbed before they touch disk.** The fingerprint harness
  (`bun run fingerprints:capture`, `fingerprints:import-har`) forwards the official client's request
  — real token included — only to the real upstream, and writes the exchange to a git-ignored
  `captures/` directory after `bluey_protocols::fingerprints::scrub` has replaced tokens, JWTs,
  API keys, account / organisation / project ids, e-mails, device and session ids, home
  directories and the user's own text and images with placeholders (`<ACCESS_TOKEN>`, `<UUID>`,
  `<EMAIL>`, `<TEXT n>`, `<BASE64 n>`). Only reviewed goldens are committed, and a test scans every
  committed fixture for secret shapes.
* **Imports are read-only.** Importing an existing sign-in from Claude Code, Codex CLI or the
  Antigravity app copies tokens into Bluey's own Keychain entry and never writes to `~/.claude`,
  `~/.codex`, Antigravity's data directory or their Keychain items.
* **One consent dialog per provider, once**, before the browser opens: what is sent, whose plan
  limits are used, that the integration is unofficial and may stop working, and what Bluey does
  when it does.
* `data_reset_all` runs every step even when one fails and reports the failures together
  (`reset_incomplete`, with the steps in `details`); it deletes `account:*` entries too once they
  exist.

## Fast path and prefetch (ADR 0010)

The speculative warm frame reuses the smart-observation frame (≤ 1.5 s old, unchanged dHash) for
⌘↵ only when screen permission is granted **and** the smart-observation setting is on; retrieval
for the current transcript question is precomputed from the transcript the user is already
recording. Nothing is captured that the user did not already enable, and no frame is sent to a
model without ⌘↵ / ⌘⇧↵. OCR moves off the critical path but keeps its retention rules: OCR text
follows the *store transcripts / screenshots* settings exactly as before.

## Frontend ⇄ backend boundary
* Every command has a typed signature in `src/lib/tauri/commands.ts`; the Rust side validates
  parameters and returns typed `BlueyError`s.
* Capabilities: `main` (HUD) gets core window/event permissions plus the Bluey commands it
  needs; `settings` additionally gets dialog/opener/autostart; `onboarding` a subset. No window
  gets `shell:allow-execute`; sidecars are spawned from Rust only.
* CSP restricts scripts to the bundle and connections to Tauri IPC only; nothing in the WebView
  talks to Clerk or to a provider — sign-in runs in the system browser and Rust completes it
  (PKCE, `state`, `nonce`, ID-token checks; the `bluey-oauth` loopback listener binds
  `127.0.0.1` only, for one request of at most 8 KB within 5 s, and unrelated `bluey://` links
  are ignored).

## Agent security (research sidecar: Gemini function calling or Claude Agent SDK)
* `tools: []` removes all built-in tools (no Bash/Read/Write/WebFetch); the only tools are the
  in-process MCP tools `exa_search`, `firecrawl_scrape`, `document_read`, allow-listed by name.
* `permissionMode` never prompts; anything not allow-listed is denied.
* `cwd` is an empty temp dir; the process runs with the app's user privileges but touches no
  files.
* `document_read` is limited to ids Rust passed in `allowedDocumentIds`, served from SQLite by
  Rust over the protocol — the sidecar has no database access.
* One process per job, `maxTurns` bound, `AbortController` cancellation, killed on app exit.

## Research privacy
Web queries are **public queries only**. `buildPublicQuery` removes names, emails, phone
numbers, employer/education details and anything sourced from resume/session documents. Private
context is merged with results locally. Example: search "software engineer interview questions
for Acme", never "John Doe, who worked at X per his resume, is interviewing at Acme".

## Logging
`tracing` with levels error/warn/info/debug/trace; production default `info`. Never logged:
API keys, auth tokens, raw audio, screenshots, resume text, transcript text (unless
`privacy.debugLogTranscripts` is enabled for local debugging), provider request bodies.

## Privacy display mode
See ADR 0006. Content protection uses `NSWindow.sharingType = .none` via Tauri; Bluey reports
platform limits honestly and does not attempt to defeat monitoring software.

## Data deletion
`data_delete_screenshots`, `data_clear_transcripts`, `data_clear_ai_cache`,
`sessions_delete(_all)`, `documents_delete(_all)` and `data_reset_all` remove rows **and** the
files they reference (frame cache), then `VACUUM`. Reset also clears Keychain entries owned by
Bluey and the Clerk session.
