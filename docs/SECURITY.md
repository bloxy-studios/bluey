# Security & Privacy Model

## Principles
1. **Nothing happens silently.** Capture and listening only start on explicit user action
   (shortcut, HUD button, menu bar) and are always visible (HUD pill, menu bar item).
2. **Secrets never reach the renderer.** API keys and the sign-in tokens are stored in the
   macOS Keychain by the Rust process (`security-framework`, ADR 0011). The WebView can only
   `set`, `has`, `delete` (and see the saved/locked/absent state of)
   the API keys listed in `SECRET_KEYS` (`src/lib/tauri/commands.ts`: provider, Exa, Firecrawl
   and agent keys); the `secrets_*` commands reject every other key (`auth:*`, `account:*`)
   before the store is touched, and the WebView never sees a token at all. The one exception is
   `secrets_allow_access`, which also takes the sign-in and subscription-account token keys the
   *Saved credentials* list shows, reads only an item the silent probe reports *locked*, and
   returns just its state.
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
| AI subscription tokens (ChatGPT / Claude / Google — ADR 0009, from the Provider Accounts PRs) | Keychain `account:<account_id>:oauth_tokens` | Rust only (`AccountsManager`): refreshed under a single-flight lock (and once more, forced, when the provider rejects an unexpired Bluey-owned token), rewritten only when a refresh changed them, injected into provider requests by the adapter, never returned to the WebView, never passed to a sidecar |
| Clerk publishable key + public OAuth client id | `VITE_CLERK_PUBLISHABLE_KEY`, `BLUEY_CLERK_OAUTH_CLIENT_ID` (public by design): the environment / `.env.local` / `.env` at startup, else the values `src-tauri/build.rs` compiled in from the same files (an explicit allowlist of public identifiers — never API keys) | Rust derives the issuer; the WebView never talks to Clerk |

Sidecars are spawned with a **cleared environment**: the helper and the research agent receive
only `PATH`/`HOME`/`TMPDIR`/`USER`/`LANG` plus the variables Rust passes explicitly
(`AgentManager::job_env` — exactly one research backend's credentials, the Exa/Firecrawl keys and
documented `BLUEY_*` knobs). Keys that `.env` loads into Bluey's own process therefore never reach
a child process that has no business with them. Log lines (file and debug stderr) are redacted
for `sk-…`, `fc-…`, `AIza…`, `Bearer …`, JWTs, `api-key` / `access_token` / `refresh_token` /
`id_token` / `client_secret` / `password` values — also inside JSON-escaped field values — and
`key=` URL queries.

Keys entered in Settings are written straight to the Keychain and the UI only shows
"Key saved" (or "locked" — saved, but macOS wants the user's OK before this build reads it —
with *Allow access*, and a confirmed *Remove key*). `.env` values (provider keys and
`EXA_API_KEY` / `FIRECRAWL_API_KEY`) are imported into the Keychain when no entry exists yet and
can be removed from disk afterwards. Release builds read `.env` files only from an explicit
`BLUEY_ENV_FILE`, never from the launch or executable directory. Nothing secret is written to SQLite or logs; the logger redacts common key
patterns (`sk-…`, `fc-…`, bearer tokens) defensively.

### Keychain access (ADR 0011)
Items live in the login keychain under the service `com.codewithabdul.bluey`
(`com.codewithabdul.bluey.dev` for debug builds, so a dev build never touches the installed
app's items). macOS trusts the *code identity* that created an item; a build with a different
identity (an ad-hoc update, a rebuild) must be approved before it reads an item's value.

| Action | Keychain access |
|---|---|
| Boot, settings save, presence flags, *Saved credentials* | Attribute-only (enumerate / exists / non-interactive probe) — never prompts |
| First use of a key or token in a process | One data read, then cached in memory (`Zeroizing`) for the process |
| Save a key, persist a refreshed token | Attribute-only delete, then add (the running build owns the new item); an unchanged value is not rewritten |
| Remove key, disconnect, sign out, reset | Attribute-only delete, status checked; local state is cleared even if the delete fails |
| *Allow access* (Settings) | The single deliberate interactive read — only of a listed item that is *locked* |
| *Import* from Claude Code / Antigravity | One read of the other app's item, only on the Import click; a denial says macOS blocked it and to click Import again and choose Allow |

A denied, cancelled or non-interactive read is reported as a *locked* credential
(`storage.keychain_*` codes), never as a missing one, and never signs the user out.

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
  `~/.codex`, Antigravity's data directory or their Keychain items. ChatGPT (PR 3a) reads
  `$CODEX_HOME/auth.json` / `~/.codex/auth.json` and does **not** refresh at import time: OpenAI
  rotates refresh tokens, so the copied session is shared with the CLI and whichever side
  refreshes first signs the other out later — the Import button says so. Claude (PR 3b) reads the
  Keychain item `Claude Code-credentials` (macOS may ask for permission), else
  `~/.claude/.credentials.json`, plus `~/.claude.json` for the account uuid — read-only, and never
  refreshed by Bluey (Anthropic rotates refresh tokens too): an imported ChatGPT or Claude session
  that expires moves to *needs sign-in*: sign in in the browser or import again.
  Google AI (PR 3c) reads the Keychain item `gemini` / `antigravity` the standalone Antigravity app
  and `agy` keep (macOS may ask), never writes it; an expired access token is renewed, which is safe
  because Google refresh tokens do not rotate.
* **The Antigravity OAuth client secret** is public in Google's shipped app but not committed here:
  the build supplies it (`BLUEY_ANTIGRAVITY_CLIENT_SECRET` in `.env.local` / `.env`, baked in by
  `src-tauri/build.rs` next to the public Clerk identifiers, or set in the environment). It is not a
  user secret, it is never logged, and a build without it simply cannot start a Google sign-in — the
  account card says so up front.
* **The per-install device id** (`accounts:device_id` in the settings table, 64 random hex) identifies
  this install in the shapers' `metadata.user_id`-style fields. It is not a secret, carries no user
  data and never leaves together with a token.
* **One consent dialog per provider, once**, before the browser opens: what is sent, whose plan
  limits are used, that the integration is unofficial and may stop working, and what Bluey does
  when it does.
* `data_reset_all` runs every step even when one fails and reports the failures together
  (`storage.reset_incomplete`, with the steps in `details`); it deletes `account:*` entries too
  once they exist.

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
  needs; `settings` additionally gets dialog/opener/autostart; `onboarding` a subset. The
  opener may open only `https://`, `http://` and `mailto:` links. No window
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
* One process per job, a turn budget and a `deadlineMs`; cancellation aborts the model call and
  in-flight Exa/Firecrawl requests, and Rust kills the process (publishing the cancellation
  itself) if the job has not ended 2 s after `research.cancel`. Killed on app exit.
* The Claude Code subprocess (Claude backend) gets no Exa/Firecrawl keys and runs with
  `DISABLE_TELEMETRY`, `DISABLE_ERROR_REPORTING` and `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC`.
* Tool output is treated as untrusted data (the system prompt says so); the report's links and
  the citation list are checked against the URLs the tools actually returned — unknown links
  are de-linked, and only sources the model used (or pages it read) are cited.

## Research privacy
Web queries are **public queries only**, on a best-effort basis. `buildPublicQuery` strips
emails, phone numbers and @handles, the signed-in user's names, and proper nouns found in the
private documents retrieved for the ask (résumé, notes, personal instructions, …); the engine
passes those terms in explicitly. It cannot recognise private facts it was never given, so the
router also keeps research off for candidate answers about the user's own experience. Private
context is merged with results locally. Example: search "software engineer interview questions
for Acme", never "John Doe, who worked at X per his resume, is interviewing at Acme".

## Logging
`tracing` with levels error/warn/info/debug/trace; production default `info`. Never logged:
API keys, auth tokens, raw audio, screenshots, resume text, transcript text (unless
`privacy.debugLogTranscripts` is enabled for local debugging), provider request bodies.
Logs are daily files in `~/Library/Logs/Bluey`; files older than 14 days are deleted at startup
and Reset all data deletes all of them.

## Privacy display mode
See ADR 0006 and its 2026-09-28 addendum. Content protection uses `NSWindow.sharingType = .none`
via Tauri, which hides Bluey only from apps that honour macOS window protection (legacy capture):
modern ScreenCaptureKit screen sharing on macOS 15+ may still show Bluey, and native menus are
never hidden, so `CaptureProtection.partial` is reported there. Bluey does not attempt to defeat
monitoring software.

## Data deletion
`data_delete_screenshots`, `data_clear_transcripts`, `data_clear_ai_cache`,
`sessions_delete(_all)`, `documents_delete(_all)` and `data_reset_all` remove rows **and** the
files they reference (frame cache). The database runs with `PRAGMA secure_delete` (freed pages
are zeroed) and FTS5 `secure-delete` (a deleted row's tokens leave the search index at once), and
each of these deletions — plus retention pruning — ends with a `wal_checkpoint(TRUNCATE)`, so the
deleted text is not left readable in `bluey.db` or `bluey.db-wal`; reset also runs `VACUUM`
and deletes the log files. Reset also deletes every Keychain item of Bluey's service — enumerated by attributes, so keys of providers removed earlier go too — and the
Clerk session; deletes never read the item first, so they never prompt.
