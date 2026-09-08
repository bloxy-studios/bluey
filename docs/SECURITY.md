# Security & Privacy Model

## Principles
1. **Nothing happens silently.** Capture and listening only start on explicit user action
   (shortcut, HUD button, menu bar) and are always visible (HUD pill, menu bar item).
2. **Secrets never reach the renderer.** API keys and the Clerk client token are stored in the
   macOS Keychain by the Rust process (`keyring`). The WebView can only `set`, `has`, `delete`.
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

Sidecars are spawned with a **cleared environment**: the helper and the research agent receive
only `PATH`/`HOME`/`TMPDIR`/`USER`/`LANG` plus the variables Rust passes explicitly
(`AgentManager::job_env` — exactly one research backend's credentials, the Exa/Firecrawl keys and
documented `BLUEY_*` knobs). Keys that `.env` loads into Bluey's own process therefore never reach
a child process that has no business with them. Log lines are redacted for `sk-…`, `fc-…`,
`AIza…`, `Bearer …`, `api-key` values and `key=` URL queries.
| Clerk client JWT | Keychain `auth:clerk:client_token` | Replayed by clerk-js at load (native mode) |
| Clerk publishable key | `VITE_CLERK_PUBLISHABLE_KEY` (public by design) | Frontend |

Keys entered in Settings are written straight to the Keychain and the UI only shows
"Key saved". `.env` values are imported into the Keychain on first run and can be removed from
disk afterwards. Nothing secret is written to SQLite or logs; the logger redacts common key
patterns (`sk-…`, `fc-…`, bearer tokens) defensively.

## Frontend ⇄ backend boundary
* Every command has a typed signature in `src/lib/tauri/commands.ts`; the Rust side validates
  parameters and returns typed `BlueyError`s.
* Capabilities: `main` (HUD) gets core window/event permissions plus the Bluey commands it
  needs; `settings` additionally gets dialog/opener/autostart; `onboarding` a subset. No window
  gets `shell:allow-execute`; sidecars are spawned from Rust only.
* CSP restricts scripts to the bundle (Clerk UI is bundled) and connections to Clerk's
  Frontend API + Tauri IPC. Provider endpoints are contacted from Rust, not from the WebView.

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
