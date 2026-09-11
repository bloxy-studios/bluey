# Provider accounts — signing in with the owner's AI subscriptions

Companion to ADR 0009 and to `docs/reference/provider-accounts-fast-path-brief.md` §2–§4c. This
is the **living record**: which providers are built, which fingerprint version ships, when each
fact was last verified against the official client or the reference implementations, the consent
copy, and the re-capture runbook. Everything with a date is re-checked on the day the code that
depends on it is written — never substituted from memory.

Bluey is a personal, single-user tool: one owner, their own paid subscriptions, their own Mac.
None of the three integrations is an offered API; all three follow the pattern of CLIProxyAPI,
vibeproxy, T3 Code, OpenClaw and the opencode auth plugins. The engineering consequences are
fingerprint parity, drift detection, an extra-usage guard and automatic fallback to the API key.

## Status by provider

| Provider | Credential | Status | Fingerprint `VERSION` / `CAPTURED_ON` | Facts verified | Lands in |
|---|---|---|---|---|---|
| Google Gemini, Foundry/Azure, Anthropic, OpenAI-compatible | API key (`provider:<id>:api_key`) | shipped | — | ADR 0007 | — |
| ChatGPT Free / Plus / Pro | Codex OAuth (`account:<id>:oauth_tokens`) | **built** (unofficial, experimental) | `codex/0.154.0` / 2026-09-11 (documented, not yet captured from the CLI) | 2026-09-11 | PR 3a |
| Claude Pro / Max | claude.ai OAuth, Claude Code wire format | specified | — / — (capture first, §4b.1) | 2026-09-11 | PR 3b |
| Google AI Pro / Ultra | Antigravity OAuth, Cloud Code `v1internal` | specified | — / — | 2026-09-11 | PR 3c |

The **accounts layer** itself landed in PR 2: the mirrored types (`bluey_core::types::accounts` ⇄
`src/lib/types/accounts.ts`), the pure rules (`bluey_core::accounts`), the `RequestShaper` trait,
`AccountsManager` with pluggable `ProviderProfile`s, the `accounts_*` commands and
`accounts.changed` / `accounts.catalog` events, `MockTransport` fake accounts with fixture
catalogs, Settings → AI → Accounts (cards, consent dialog, import button), the onboarding
branch, the HUD *Reconnect* / *Use API key instead* recoveries, the runtime flag
`settings.experimental.subscriptionAccounts` and the Cargo feature `subscription-accounts`. ChatGPT
flipped to *built* in PR 3a (`src-tauri/src/accounts/chatgpt.rs`, `src-tauri/src/ai/providers/chatgpt.rs`,
`bluey_protocols::codex`); Claude and Google AI follow in PR 3b / 3c — until then their profiles are
placeholders that answer `account.provider_pending`.

The tables below are also **data**: `bluey_protocols::fingerprints::{codex, claude_code, antigravity}`
hold `VERSION` / `CAPTURED_ON`, the comparison rules and the *documented* capture per endpoint
(generated into `tests/fixtures/fingerprints/<provider>/documented/`). `bun run fingerprints:capture
<provider>` records the official client through a local proxy, `bun run fingerprints:diff <provider>`
diffs that capture against the documented fingerprint and the blessed golden (PR 2b), and
`accounts_probe_fingerprint` (dev) asks the provider whether Bluey's own request is still billed to
the plan — together they keep the two right-hand columns true after every official-client release.

## How a subscription account works

1. **Connect** — Settings → AI → Accounts → *Connect*, or onboarding → *Use a subscription I
   already pay for*. The consent dialog (below) is shown once per provider; then the system
   browser opens on the vendor's authorize page and Rust completes the flow (PKCE; loopback
   listener, device code or manual `code#state` paste depending on the provider).
2. **Identity** — plan tier, e-mail and account/project ids are resolved (Codex: ID-token claims;
   Claude: profile endpoint or rate-limit headers; Antigravity: `loadCodeAssist`).
3. **Catalog** — the models the subscription exposes are fetched or probed, cached (10 min; 24 h
   for probed catalogs) and mapped onto Bluey's roles by capability; never a model the catalog did
   not return.
4. **Requests** — the adapter resolves a `CredentialSource::OAuth` per request (refresh under a
   single-flight lock 60 s before expiry) and applies the provider's `RequestShaper` last, so the
   request matches the official client's captured shape.
5. **Fallback** — `NeedsReauth`, `Unavailable` or `RateLimited{until}` routes the role to the
   API-key provider (then the router's chain) and the HUD shows *Reconnect* / *Use API key
   instead*; a rate-limited account is skipped until its window resets.

## Verified wire facts

Each subsection is a dated table. `VERIFY` marks a value the reference implementations agree on
but that only a live capture of the official client can settle for today's build.

### ChatGPT — Codex OAuth

Verified 2026-09-11 against `openai/codex` @ `ab95cd4` (paths relative to `codex-rs/`), stable
release `rust-v0.154.0` (2026-09-09), `EvanZhouDev/openai-oauth` @ `ec7dab2`,
`opencoredev/login-with-chatgpt` @ `3befb7f`, `router-for-me/CLIProxyAPI` @ `09a29bd`, and the
official Codex auth docs. Full pass with every file:line:
`docs/reference/verification-2026-09-11/codex.md`.

| Fact | Value (2026-09-11) | Source | Notes |
|---|---|---|---|
| Client id | `app_EMoamEEZ73f0CkXaXp7hrann` | `login/src/auth/manager.rs:1724` | as the brief |
| Authorize | `https://auth.openai.com/oauth/authorize` with `response_type=code`, `client_id`, `redirect_uri`, `scope`, `code_challenge`, `code_challenge_method=S256`, `id_token_add_organizations=true`, `codex_cli_simplified_flow=true`, `state` (32 random bytes, base64url), `originator=codex_cli_rs` | `login/src/server.rs:576-612` | send all of them, in this order |
| Scope | `openid profile email offline_access api.connectors.read api.connectors.invoke` | `login/src/server.rs:590-591` | **DELTA** — the brief and all three reference implementations use the 4-scope string. Bluey sends the CLI's string; whether 4-scope tokens still work is VERIFY |
| Redirect | `http://localhost:1455/auth/callback`; allow-listed fallback port **1457**; path fixed; listener on `127.0.0.1` | `login/src/server.rs:60-62,637-695` | Bluey order: 1455 → 1457 → device code |
| PKCE | S256; verifier 64 random bytes base64url (86 chars) | `login/src/pkce.rs:12-27` | |
| Code exchange | `POST https://auth.openai.com/oauth/token`, **form-encoded**: `grant_type=authorization_code`, `code`, `redirect_uri`, `client_id`, `code_verifier` → `id_token`, `access_token`, `refresh_token` (all three required) | `login/src/server.rs:827-876` | `expires_in` presence VERIFY |
| Refresh | same endpoint, **JSON** `{"client_id","grant_type":"refresh_token","refresh_token"}` → optional `id_token` / `access_token` / `refresh_token` (keep old values when missing); refresh when JWT `exp` ≤ now + 5 min (or `last_refresh` > 8 days) | `login/src/auth/manager.rs:1599-1721,2959-2981` | permanent failures → `NeedsReauth`: `refresh_token_expired`, `refresh_token_reused`, `refresh_token_invalidated`, HTTP 400 `invalid_grant`, any 401. Revoke: `https://auth.openai.com/oauth/revoke` |
| Device flow | `POST https://auth.openai.com/api/accounts/deviceauth/usercode` JSON `{"client_id"}` → `{device_auth_id, user_code, interval}` (interval is a *string* of seconds); user opens `https://auth.openai.com/codex/device`; poll `POST …/api/accounts/deviceauth/token` `{device_auth_id, user_code}` — 403/404 = pending, 15 min cap → `{authorization_code, code_challenge, code_verifier}` → normal code exchange with `redirect_uri=https://auth.openai.com/deviceauth/callback` and the server's verifier | `login/src/device_code_auth.rs` | **DELTA** — `/codex/device` is the user page, not the API. 404 on `usercode` = device login disabled |
| Identity | claims under `https://api.openai.com/auth` in the ID token (and the access token): `chatgpt_account_id`, `chatgpt_plan_type`, `chatgpt_user_id`, `chatgpt_account_is_fedramp`; e-mail: top-level `email`, else `https://api.openai.com/profile`.`email` | `login/src/token_data.rs:71-117`; plan values `protocol/src/auth.rs:66-126` (`free`, `go`, `plus`, `pro`, `prolite`, `team`, `business`, `enterprise`, `edu`, …) | payload decoded for display only; no signature check. `account_id` = `chatgpt_account_id`, sent on every backend request |
| Backend | `https://chatgpt.com/backend-api/codex` — `POST /responses` (SSE) and `GET /models` | `model-provider-info/src/lib.rs:43`; `codex-api/src/endpoint/responses.rs` | alias host `chat.openai.com`; staging `chatgpt-staging.com` |
| Headers | `Authorization: Bearer <access_token>` · `chatgpt-account-id: <account_id>` · `originator: codex_cli_rs` · `User-Agent: codex_cli_rs/0.154.0 (Mac OS <ver>; arm64) <terminal>` · `version: 0.154.0` · `Accept: text/event-stream` · `Content-Type: application/json` · `session-id`, `thread-id`, `x-client-request-id` (uuids) · `X-OpenAI-Fedramp: true` only for FedRAMP accounts | `model-provider/src/bearer_auth_provider.rs:35-47`; `login/src/auth/default_client.rs:164-188`; `codex-api/src/requests/headers.rs:5-14` | **DELTA** — the CLI sends **no `OpenAI-Beta` header** on HTTP (`responses=experimental` appears nowhere in the repo; WebSocket only uses `responses_websockets=2026-02-06`). `session_id`/`conversation_id` are legacy names. Echo a received `x-codex-turn-state` response header back as a request header |
| Body | `model` (slug) · `instructions` · `input` (full history each time) · `tool_choice: "auto"` · `parallel_tool_calls` · `reasoning: {effort: none\|low\|medium\|high\|xhigh, summary: auto\|concise\|detailed\|none}` · `store: false` · `stream: true` · `include: ["reasoning.encrypted_content"]` · `prompt_cache_key: <session-id>` · `text: {verbosity: low\|medium\|high, format: {type: json_schema, strict, schema, name}}` | `codex-api/src/common.rs:259-285`; `core/src/client.rs:852-889` | never `previous_response_id`, `max_output_tokens`, `temperature`. `reasoning` must be present (Codex models always reason) |
| `instructions` | the CLI sends the model's `model_messages.instructions_template` (13–21 KB Codex prompt) from the `/models` catalog; openai-oauth and CLIProxyAPI send `""` and report working streams; one June 2026 article claims server-side validation | `core/src/client.rs:794-835`; `protocol/src/openai_models.rs:534-552` | **VERIFY by replay in PR 3a** (send `""`, then a one-line prompt, then the template). Ship `""` + Bluey's prompt as a `developer` item if accepted — ~4–5 K fewer prefill tokens per request; else the template |
| Images | `{"type":"input_image","image_url":"data:<mime>;base64,…","detail":"high"}`; `detail` ∈ `auto\|low\|high\|original`; CLI default `high`, rejects `low` client-side | `protocol/src/models.rs:878-925`; `core/src/image_preparation.rs` | Bluey: `auto` for chat screens, `high` for dense text; `low` only if the replay test shows the server accepts it. Byte/pixel limits live in `codex-utils-image` — VERIFY |
| SSE events | handle `response.created`, `response.output_item.added/done`, `response.output_text.delta`, `response.reasoning_summary_text.delta/done`, `response.completed` (usage: `input_tokens`, `input_tokens_details.cached_tokens`, `output_tokens`, `output_tokens_details.reasoning_tokens`, `total_tokens`), `response.incomplete` → error, `response.failed` → `error.code` (`context_length_exceeded`, `insufficient_quota`, `usage_not_included`, `rate_limit_exceeded`, `server_is_overloaded`, `slow_down`, `invalid_prompt`, …) | `codex-api/src/sse/responses.rs:353-522` | EOF without `response.completed` = `network.stream`, as in the Gemini adapter |
| Catalog | `GET https://chatgpt.com/backend-api/codex/models?client_version=0.154.0` with the same headers → `{"models":[{slug, display_name, description, default_reasoning_level, supported_reasoning_levels[{effort, description}], visibility: list\|hide\|none, supported_in_api, priority, minimal_client_version, context_window, input_modalities[text\|image\|audio], model_messages.instructions_template, available_in_plans, …}]}` | `codex-api/src/endpoint/models.rs:31-79`; `protocol/src/openai_models.rs:398-500` | sort by `priority`, offer `visibility == "list"`; vision = `"image" ∈ input_modalities`. Constant `CODEX_CLIENT_VERSION = "0.154.0"` — a stale value makes every model "not supported"; bump with each Codex release. Snapshot in the CLI bundle: `gpt-6-astra`, `gpt-5.6-sol/-terra/-luna`, `gpt-5.5` listed; `gpt-5.4`, `codex-auto-review`, `gpt-daybreak-*` hidden |
| Rate limits | 429 body `{"error":{"type":"usage_limit_reached","plan_type","resets_at":<unix s>}}` (CLIProxyAPI falls back to `resets_in_seconds`); also `usage_not_included`, `insufficient_quota`; headers `x-codex-primary-used-percent`, `x-codex-primary-window-minutes`, `x-codex-primary-reset-at`, `x-codex-secondary-*`, `x-codex-credits-{has-credits,unlimited,balance}`, `x-codex-active-limit`, `x-codex-rate-limit-reached-type` | `codex-api/src/api_bridge.rs:133-181`; `codex-api/src/rate_limits.rs:57-102` | **DELTA** — `-reset-at`, not `-reset-after-seconds`. `RateLimited{until: resets_at ?? primary-reset-at, window: "<window-minutes> min"}` |
| Other errors | 401 → refresh once → `NeedsReauth`; 403 not recoverable — Cloudflare "blocked" body = region block, `misalignment_policy_violation` → `Unavailable{PolicyBlocked}`; 400 `cyber_policy` / invalid image / else `ai.invalid_request`; 503 `server_is_overloaded\|slow_down` and other 5xx → retry before first byte (CLI: 4 attempts, never retries 429 at the transport) | `codex-api/src/api_bridge.rs:75-132,224-244` | |
| Local store | `~/.codex/auth.json` (0600): `{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token","access_token","refresh_token","account_id"},"last_refresh":"<RFC 3339>"}`; optional keyring mode: macOS Keychain service `Codex Auth`, account `cli|<first 16 hex of sha256(codex_home)>`, same JSON | `login/src/auth/storage.rs:39-65,230-406`; `config/src/types.rs:106-119` | import reads either, copies, never writes back |
| Capture knob | the CLI honours `HTTPS_PROXY` / system proxy with a custom-CA path; `chatgpt_base_url` in `~/.codex/config.toml` pins the backend base URL | `login/src/auth/default_client.rs:305-310`; Codex auth docs | for the re-capture runbook |
| Policy | no OpenAI document authorises or prohibits third-party use of the Codex client id; official auth docs are silent; Sam Altman's 2026-05-01 post endorsed using a ChatGPT subscription inside OpenClaw (a third party using this flow) | `developers.openai.com/codex/auth`; press quotes 2026-05-03/04 | engineering input only: kill switch + drift detection |

**What Bluey sends (PR 3a, `bluey_protocols::codex`).** Sign-in: the CLI's six-scope string and
parameter order, PKCE with a 64-byte verifier, loopback `1455` → `1457` → device code
(`prefer_device_code` skips the ports). Requests: every header of the *Headers* row from the
`fingerprints::codex` constants, `User-Agent` ending in `Bluey` where the CLI puts the terminal
name, no `OpenAI-Beta`; body per the *Body* row with `instructions` = the model's
`instructions_template` from the catalog and Bluey's own prompt as a `role: "developer"` input
item (the `""` / short-prompt replay of the `instructions` row is still open — the first real
probe settles it and may let Bluey drop ~4–5 K prefill tokens), `reasoning.effort` clamped to
the model's `supported_reasoning_levels` from Bluey's level + latency budget, `text.verbosity`
`low` for ultra-fast else `medium`, `input_image.detail: "auto"` until PR 4b's hints,
`prompt_cache_key` = one UUID per Bluey session. Catalog: `client_version` pinned to
`fingerprints::codex::CLIENT_VERSION`; a 404 or an empty list → `Unavailable{CatalogUnavailable}`.
Errors: `bluey_protocols::codex::map_error` — 401 → `NeedsReauth`, 403 → `PolicyBlocked`
(`FingerprintDrift` when the body blames the credential / client), 429 → `RateLimited{until,
window}` from `resets_at` / `resets_in_seconds` / `x-codex-primary-*` (`usage_not_included` →
`PolicyBlocked`, `insufficient_quota` → rate-limited without a reset), 400 "unsupported parameter"
→ `FingerprintDrift`, 404 on `/models` → `CatalogUnavailable`. A test diffs the shaper's output
against the documented capture (`fingerprints:diff` semantics: no drift). Import copies
`$CODEX_HOME/auth.json` / `~/.codex/auth.json` without refreshing — the copied session is shared
with the CLI and refresh-token rotation may later sign one side out.

### Claude Pro / Max — claude.ai OAuth, Claude Code wire format

Verified 2026-09-11 against `router-for-me/CLIProxyAPI` @ `09a29bd` (fingerprint aligned to Claude
Code 2.1.258; paths below are its), `automazeio/vibeproxy` @ `6280856` (bundles CLIProxyAPI),
`shahidshabbir-se/opencode-anthropic-oauth` @ `a6e7a85`, `kristianvast/hermes-claude-auth` @
`6525928` (+ PRs #10, #15), `griffinmartin/opencode-claude-auth` (2.1.257), pi-ai @ `d12cd92`,
`anthropics/claude-code` CHANGELOG (current CLI **2.1.268**), the Anthropic TypeScript SDK, and the
official Messages / rate-limit / error / caching / authentication / legal pages. Full pass:
`docs/reference/verification-2026-09-11/claude.md`. Every wire fact below is a third-party
reconstruction from proxy captures of the real binary — the §4b.1 capture with the owner's CLI is
the golden fixture, not these tables.

| Fact | Value (2026-09-11) | Source | Notes |
|---|---|---|---|
| Client id | `9d1c250a-e61b-44d9-88ed-5944d1962f5e` | `internal/auth/claude/anthropic_auth.go:33`; all refs | as the brief |
| Authorize | `https://claude.ai/oauth/authorize` with `code=true`, `client_id`, `response_type=code`, `redirect_uri`, `scope`, `code_challenge` (S256), `code_challenge_method=S256`, `state` | `anthropic_auth.go:324-333` | the manual flow may use `https://claude.com/cai/oauth/authorize` (307 → same); `code=true` is required |
| Scopes | `org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload` (six — the real 2.1.267 authorize URL in claude-code #93216 and pi-ai) | pi-ai `auth/oauth/anthropic.ts:36-37`; CLIProxyAPI PR #4830 | CLIProxyAPI's loopback constant sends five (no `org:create_api_key`). Bluey sends six; VERIFY by capture |
| Redirect | loopback `http://localhost:54545/callback` (Claude Code's fixed port; pi-ai uses 53692, so the port is client-chosen); manual paste `https://platform.claude.com/oauth/code/callback`, format `code#state` | `anthropic_auth.go:34,348-354`; opencode-anthropic-oauth `oauth.ts:11-13` | Bluey order: 54545 → random loopback port → manual paste. **Day-one risk:** claude-code #93216 (open, 2026-09-09) reports the server rejecting even the correct manual redirect for this client id — re-check before PR 3b ships |
| Token endpoint | `https://platform.claude.com/v1/oauth/token` — JSON `{grant_type:"authorization_code", code, redirect_uri, client_id, code_verifier, state}` with axios-shaped headers (`Content-Type: application/json`, `Accept: application/json, text/plain, */*`, `User-Agent: axios/1.15.2`); refresh JSON `{client_id, grant_type:"refresh_token", refresh_token, scope}` → `access_token`, `refresh_token`, `token_type`, `expires_in` (~36 000 s), `organization{uuid,name}`, `account{uuid,email_address}` | `anthropic_auth.go:27-28,146-153,211-221,526-531` | **DELTA** — the brief's `console.anthropic.com/v1/oauth/token` has been behind a Cloudflare challenge for non-browser POSTs since ~2026-02 (CLIProxyAPI commit `ef5901c`); `platform.claude.com` since the 2.1.220 alignment (`f3e25ab`, 2026-08-02). Presence of `account`/`organization` in the token response is VERIFY — refetch the profile anyway |
| Token prefixes | access `sk-ant-oat01-…`, refresh `sk-ant-ort01-…` | gitleaks #2158/#2159 | redaction patterns |
| Identity / plan | `GET https://api.anthropic.com/api/oauth/profile` with `Authorization: Bearer`, `anthropic-beta: oauth-2025-04-20` → `account{uuid,email,has_claude_pro,has_claude_max}`, `organization{uuid,name,rate_limit_tier,organization_type,seat_tier,subscription_status}`; tiers `default_claude_pro` / `default_claude_max_5x` / `default_claude_max_20x`; `organization_type` `claude_pro` / `claude_max` / `claude_team` / `claude_enterprise` | `anthropic_auth.go:29,156-165,211-239`; meridian PR #795/#803 | companion `GET /api/oauth/claude_cli/roles` fires after exchange (opaque). `GET /api/oauth/usage` (undocumented; `five_hour` / `seven_day` utilisation and `resets_at`) is best-effort and often 429s |
| Transport | `Authorization: Bearer <access_token>` — **only**; an OAuth token in `x-api-key` returns 401 `authentication_error` | `claude_executor_request.go:867-874`; hermes-agent PR #51068 (2026-06-22) | the brief's "April x-api-key report" was this bug, not a transport change |
| URL / version | `POST https://api.anthropic.com/v1/messages?beta=true` (`/v1/messages/count_tokens?beta=true`); `anthropic-version: 2023-06-01` | `claude_executor_execute.go:32`; SDK `messages.ts:120,206` | |
| Betas — always on (wire order, CLI 2.1.258) | `claude-code-20250219`, `oauth-2025-04-20`, `interleaved-thinking-2025-05-14`, `redact-thinking-2026-02-12`, `thinking-token-count-2026-05-13`, `context-management-2025-06-27`, `prompt-caching-scope-2026-01-05` | `claude_executor_request.go:56-176` | **DELTA** — three more than the brief |
| Betas — conditional | `mid-conversation-system-2026-04-07` (models that accept a `role: "system"` turn — Bluey needs it), `effort-2025-11-24` (effort-capable model with thinking on; never for Haiku), `fallback-credit-2026-06-01` and `extended-cache-ttl-2025-04-11` (OAuth), `structured-outputs-2025-12-15` (with `output_config.format`), `context-1m-2025-08-07` (contested — griffinmartin never sends it; CLIProxyAPI only when asked on a 1M model) | `claude_executor_request.go:88-176`; griffinmartin `betas.ts` | never send `context-1m` by default; on a long-context 400, drop it and retry once |
| Headers | `User-Agent: claude-cli/2.1.258 (external, cli)` (native regex `^claude-cli/\d+\.\d+\.\d+\s+\(external,\s*<entrypoint>\)$`) · `x-app: cli` · `anthropic-dangerous-direct-browser-access: true` · `X-Claude-Code-Session-Id: <uuid per conversation>` · `x-client-request-id: <uuid per request>` (api.anthropic.com only) · `X-Stainless-Lang: js`, `X-Stainless-Package-Version: 0.112.1`, `X-Stainless-OS: MacOS`, `X-Stainless-Arch: arm64`, `X-Stainless-Runtime: node`, `X-Stainless-Runtime-Version: v26.3.0`, `X-Stainless-Retry-Count: 0`, `X-Stainless-Timeout: 600` · `Accept: application/json` (native, even when streaming to api.anthropic.com) · `Accept-Encoding: gzip, deflate, br, zstd` | `claude_executor_request.go:1021-1153`; `helps/claude_device_profile.go:23-27` | the Stainless package/runtime versions track the SDK bundled in the CLI (0.81.0 at 2.1.117 → 0.112.1 at 2.1.258) — VERIFY for 2.1.268 |
| `system[]` — exactly two blocks | `system[0]` text `x-anthropic-billing-header: cc_version=<ver>.<3hex>; cc_entrypoint=cli; cch=<5hex>;` · `system[1]` text **`You are Claude Code, Anthropic's official CLI for Claude.`** with `cache_control: {type: "ephemeral", ttl: "1h"}` (OAuth; native default has no ttl) · **nothing else** — the caller's own system prompt is relocated to a mid-conversation `role: "system"` message (with `mid-conversation-system-2026-04-07`) or a `<system-reminder>` at the top of the first user message on legacy models | `claude_executor_request.go:243,290-299,1451-1453,1518`; griffinmartin `transforms.ts`; official LLM-gateway protocol doc ("a merged block starting with the attribution header is treated as attribution in its entirety") | **DELTA ×3** — (a) the identity text for the `cli` entrypoint is still the "Claude Code" string (the *"You are a Claude agent, built on Anthropic's Claude Agent SDK."* text belongs to the `sdk` / `claude-vscode` entrypoints; hermes retracted its 2.1.117 claim in v1.7.0); (b) billing block first, identity second; (c) third-party system content inside `system[]` is a documented extra-usage trigger (hermes #79760) — Bluey's prompt never goes there |
| `cc_version` build suffix | `.<3hex>` = `SHA256("59cf53e54c78" + firstUserText[4] + firstUserText[7] + firstUserText[20] + version)[:3]` (missing indices padded with `"0"`) | CLIProxyAPI `computeFingerprint`; griffinmartin `computeVersionSuffix`; hermes | three references agree |
| `cch` | **disputed**: griffinmartin / hermes `SHA256(firstUserText)[:5]`; CLIProxyAPI `xxHash64(seed 0x4D659218E32A3268, normalised final body) & 0xFFFFF` as five hex, with `max_tokens` / `fallbacks` excluded and `model` emptied; `00000` = unsigned placeholder; omitted on sdk/vscode entrypoints | `claude_signing.go:20,214-236`; griffinmartin `signing.ts`; hermes `_compute_cch` | **the least-settled fact** — the §4b.1 capture diffs the real `cch` against both algorithms; a wrong `cch` alone does not appear to flip billing (griffinmartin ships the simple hash and works), identity + UA + entrypoint parity does |
| `metadata.user_id` | JSON string `{"device_id":"<64 lowercase hex, stable per install>","account_uuid":"<profile account.uuid>","session_id":"<uuid>"}` — key order as shown | `claude_executor_request.go:405-452`; openclaw PR #61 ("primary tell") | `account_uuid` is not in the credentials file — profile endpoint or `~/.claude.json` `oauthAccount.accountUuid` |
| Tools / thinking | tools `mcp__<server>__<tool>` (PascalCase built-in names; lowercase names are flagged); adaptive models: `thinking: {type: "adaptive", display: "summarized"}` + `output_config.effort` (`low`…`max`), dropped when `tool_choice` is forced, effort never for Haiku; `context_management: {edits: [{type: "clear_thinking_20251015", keep: "all"}]}` when thinking is on; `max_tokens` required (64 000 Sonnet/Haiku 4.5, 128 000 Opus 5 / Sonnet 5 / Fable 5.1) | `claude_executor_request.go:553-567,931,1135,1400,2139`; pi-ai `buildParams` | Fable 5.1 additionally carries `fallbacks: [{model: "claude-opus-5"}]` |
| Catalog | `GET https://api.anthropic.com/v1/models?limit=100` with the OAuth Bearer, `anthropic-version` and `anthropic-beta: oauth-2025-04-20` returns 200 → `{data: [{id, display_name, created_at}], has_more}` | community reports (hivecommons #2524, gents #1390) | `CatalogSource::Endpoint`; the official page only shows the `x-api-key` example — VERIFY once. Current ids: `claude-opus-5`, `claude-opus-4-8` / `4-7` / `4-6` / `4-5-20251101`, `claude-sonnet-5`, `claude-sonnet-4-6`, `claude-sonnet-4-5-20250929`, `claude-haiku-4-5-20251001`, `claude-fable-5-1`, `claude-fable-5`. Presets: `default`/`vision` `claude-sonnet-5`, `reasoning` `claude-opus-5`, `fast` `claude-haiku-4-5-20251001` |
| Extra-usage signals | 400 `invalid_request_error` `Third-party apps now draw from your extra usage, not your plan limits. […] Add more at claude.ai/settings/usage and keep going.` (since **2026-04-04**) · `You're out of extra usage. Add more at claude.ai/settings/usage and keep going.` · bare 429 `{"type":"rate_limit_error","message":"Error"}` **without** unified headers = the third-party detection response (retrying unchanged never succeeds) | claude-code #45013/#45069/#45098; hermes `anthropic_billing_bypass.py:29-40` | all three are stop signals (drift table) |
| Long-context signal | 400/429 `Extra usage is required for long context requests.` | claude-code #28927, #42616 | entitlement check on 1M requests, distinct from the classifier — drop `context-1m` and retry once |
| Rate-limit headers (subscription) | `anthropic-ratelimit-unified-status`, `-5h-status`, `-7d-status`, `-7d_oi-status` (`allowed` / `allowed_warning` / `rejected`); `-5h-reset`, `-7d-reset`, `-7d_oi-reset`, `-reset` (unix seconds or RFC 3339); `-5h-utilization`, `-7d-utilization` (0–1); `anthropic-ratelimit-unified-representative-claim` (`five_hour` / `seven_day`) | `helps/claude_ratelimit.go:22-39` | `7d_oi` is the model-scoped (Fable) weekly window. API keys use the `anthropic-ratelimit-{requests,tokens,…}-*` set instead |
| Other errors | 401 `authentication_error` → refresh → `NeedsReauth`; 529 `overloaded_error` → retry before first byte; every response carries `request-id: req_…` | official errors page | |
| Local store | macOS Keychain generic password, service `Claude Code-credentials`, account = the macOS username (sometimes `default` / `unknown`; extra entries `Claude Code-credentials-<8hex>` per config dir), payload `{"claudeAiOauth":{"accessToken","refreshToken","expiresAt":<ms>,"scopes":[…],"subscriptionType":"pro"\|"max","rateLimitTier":"default_claude_max_5x","refreshTokenExpiresAt"}}`; fallback `~/.claude/.credentials.json` (0600; `CLAUDE_CONFIG_DIR` overrides); `account_uuid` / e-mail in `~/.claude.json` `oauthAccount` | griffinmartin `keychain.ts`; official `authentication.md`; hermes-agent #83338 | read-only import; `rateLimitTier` in the stored token can lag the org — prefer the profile call |
| Policy timeline | 2026-01-09 third-party OAuth blocked · 2026-02-19 docs: tokens are for Claude Code / Claude.ai only · **2026-04-04** third-party traffic routed to extra usage (Boris Cherny; "applies to all third-party harnesses") · 2026-04-08 false-positive wave hit the official client · 2026-06-15 Agent SDK monthly credit announced then paused — "third-party app usage still draw[s] from your subscription's usage limits" · legal page: third-party developers "may not … route requests through Free, Pro, or Max plan credentials"; enforcement "without prior notice" | press 2026-04-03/04; support.claude.com 15036540; code.claude.com legal-and-compliance | engineering inputs: fingerprint parity + the extra-usage guard |
| Capture knob | `ANTHROPIC_BASE_URL=http://127.0.0.1:<port>` redirects the real CLI to the capture proxy | official env-vars doc | for the re-capture runbook; the current CLI is 2.1.268 |

### Google AI Pro / Ultra — Antigravity OAuth, Cloud Code `v1internal`

Verified 2026-09-11 against `router-for-me/CLIProxyAPI` @ `09a29bd` (2026-09-10; the maintained
reference — paths below are its), `NoeFabris/opencode-antigravity-auth` @ `16e0056` (archived
2026-08-27), `firdyfirdy/antigravity-auth` @ `2beb45c`, `lbjlaq/Antigravity-Manager` @ `85fb4fe`
(v4.7.0, Rust/Tauri), `google-gemini/gemini-cli` @ `ed2ac40` (official Code Assist baseline),
Google's Antigravity terms and deprecation pages. Full pass:
`docs/reference/verification-2026-09-11/antigravity.md`. The three maintained references
**disagree** on the User-Agent family and the generation host; a proxy capture of the real app is
the §4c.1 first step of PR 3c and settles both.

| Fact | Value (2026-09-11) | Source | Notes |
|---|---|---|---|
| OAuth client | id `1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com`, secret `GOCSPX-… (public in the shipped app; copy from CLIProxyAPI `internal/auth/antigravity/constants.go:7` on the day — not committed here)` (public in the shipped app; all four references) | `internal/auth/antigravity/constants.go:6-7` | gemini-cli's client (`681255809395-…`) is a different client whose consumer path is shut down — never use it |
| Scopes | `https://www.googleapis.com/auth/cloud-platform` `…/userinfo.email` `…/userinfo.profile` `…/cclog` `…/experimentsandconfigs` — five, space-joined | `constants.go:12-18`; plugin `constants.ts:14-20` | **DELTA** — the brief's three-scope list is gemini-cli's |
| Authorize | `https://accounts.google.com/o/oauth2/v2/auth` with `access_type=offline`, `prompt=consent`, `response_type=code`, `client_id`, `redirect_uri`, `scope`, `state`; PKCE S256 optional (plugin sends it, CLIProxyAPI does not) — Bluey sends it | `auth.go:141-155`; plugin `oauth.ts:92-114` | |
| Redirect | `http://localhost:51121/oauth-callback` — fixed port 51121 in every reference | `constants.go:8`; `sdk/auth/antigravity.go:208-244` | whether other loopback ports are accepted is VERIFY; `port_in_use` → clear error (no device-code flow exists here) |
| Token endpoint | `https://oauth2.googleapis.com/token`, form-encoded; exchange `code`, `client_id`, `client_secret`, `redirect_uri`, `grant_type=authorization_code` (+ `code_verifier`); refresh `client_id`, `client_secret`, `grant_type=refresh_token`, `refresh_token` → `access_token`, `refresh_token`, `expires_in`, `token_type` | `auth.go:158-166`; `antigravity_executor_auth.go:149-162` | refresh **includes the client secret**; CLIProxyAPI refreshes 30 min before expiry with `User-Agent: Go-http-client/2.0` ("real Antigravity uses Go's default UA for refresh"); `invalid_grant` → `NeedsReauth` |
| Identity | `GET https://www.googleapis.com/oauth2/v2/userinfo?alt=json` → `email`; plan from `loadCodeAssist` | `constants.go:24` | |
| `loadCodeAssist` | `POST https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist` (**prod** host) body `{"metadata":{"ideType":"ANTIGRAVITY"}}`; headers `Authorization`, `Accept: */*`, `Content-Type: application/json`, `User-Agent` — no `X-Goog-Api-Client` → `cloudaicompanionProject`, `currentTier{id,name,…}`, `allowedTiers[]{id,isDefault,userDefinedCloudaicompanionProject}`, `ineligibleTiers[]{reasonCode,validationUrl,…}`, `paidTier{…, availableCredits[{creditType:"GOOGLE_ONE_AI",creditAmount,minimumCreditAmountForUsage}]}` | `auth.go:79-139,251-268`; gemini-cli `types.ts:78-142` | documented tier ids are only `free-tier`, `legacy-tier`, `standard-tier`; **the Pro / Ultra tier ids are VERIFY** — label by matching `paidTier`/`currentTier` name or id (as Antigravity-Manager does) and record the real ids on first connect. `ineligibleTiers[].reasonCode == VALIDATION_REQUIRED` → show `validationUrl` |
| `onboardUser` | `POST https://daily-cloudcode-pa.googleapis.com/v1internal:onboardUser` body `{"tier_id":"<tier>","metadata":{"ide_type":"ANTIGRAVITY","ide_version":"<ver>","ide_name":"antigravity"}}` (snake_case); LRO: re-POST until `done`, project = `response.cloudaicompanionProject.id`; headers add `X-Goog-Api-Client: gl-node/22.21.1` | `auth.go:85-91,317-405` | only when `loadCodeAssist` returns no project; Workspace/enterprise accounts need a user-supplied project id (CLIProxyAPI made it mandatory 2026-05-07) |
| Generation | `POST <host>/v1internal:streamGenerateContent?alt=sse` (`:generateContent` for probes, `:countTokens`); body `{"model","project","request":{contents, systemInstruction, generationConfig{thinkingConfig…}, tools, toolConfig, "sessionId"}, "userAgent":"antigravity", "requestType":"agent", "requestId":"agent-<uuid>"}`; `request.safetySettings` deleted; Claude models need `toolConfig.functionCallingConfig.mode = "VALIDATED"`; response envelope `{"response": GenerateContentResponse, "traceId"}` in `data:` SSE lines | `antigravity_executor_request.go:36-102,459-542`; gemini-cli `converter.ts:77-90` | **DELTA** — wrapper extras and envelope. CLIProxyAPI also strips `maxOutputTokens` for non-Claude models (reason not documented — VERIFY whether the pool accepts it; the fast path wants it) |
| Host | CLIProxyAPI: `https://daily-cloudcode-pa.googleapis.com` (no `.sandbox.`) for generation and `onboardUser`, prod for `loadCodeAssist`, **no cross-host cascade** (removed 2026-08-25, #5209); Antigravity-Manager and the archived plugin use `daily-cloudcode-pa.sandbox.googleapis.com`; autopush was already dead in 2025-12 | `antigravity_executor.go:33-35`; commit `adf05298` | **DELTA** — the brief's prod → daily → autopush cascade is dropped; one host per account, configurable, default CLIProxyAPI's until the capture says otherwise |
| Headers on generation | exactly `Content-Type: application/json`, `Authorization: Bearer <token>`, `User-Agent` — **no** `X-Goog-Api-Client`, `Client-Metadata`, `X-Goog-QuotaUser`, `X-Client-Device-Id`, `x-goog-user-project` (the last one *causes* 403s); HTTP/1.1 **without ALPN** ("native Antigravity never uses h2"); one connection pool per account | `antigravity_executor_request.go:117-135`; `antigravity_executor.go:240-259,907-947`; plugin CHANGELOG 1.5.0 (2026-02-10) | **DELTA** — the brief's header set is what the plugin sent *before* 2026-02-10 and removed. Bluey uses a dedicated `http1_only` client per Google account |
| User-Agent | disputed: CLIProxyAPI `antigravity/hub/2.12.2 darwin/arm64` (version from the Hub update manifest `…/manifest/latest-arm64-mac.yml`, cached 6 h, floor `2.9.1` — "Cloud Code rejects newer models for clients below 2.9.0"); Antigravity-Manager `Antigravity/<ver> (Macintosh; Intel Mac OS X 10_15_7) Chrome/132.0.6834.160 Electron/39.2.3` + `x-client-name`, `x-client-version`, `x-machine-id`, `x-vscode-sessionid` | `misc/antigravity_version.go:18-33,201-250`; AM `constants.rs:10-13,179-223` | **VERIFY by capture**. Until then Bluey follows CLIProxyAPI and reads the version from the manifest at connect time (cached), with the floor as fallback |
| System instruction | CLIProxyAPI ships **without** the "You are Antigravity, a powerful agentic AI coding assistant…" text (commented out); the plugin and Antigravity-Manager still prepend it as a `role: "user"` `systemInstruction` part | `antigravity_executor.go:47,104-115`; AM `mappers/claude/request.rs:905-960` | not required as far as any reference shows; Bluey does not inject it by default and A/Bs it in `accounts_probe_fingerprint` |
| Catalog | `POST <host>/v1internal:fetchAvailableModels` body `{"project":"<id>"}` → `{"models":{"<id>":{displayName, maxTokens, maxOutputTokens, quotaInfo{remainingFraction, resetTime}}}, "webSearchModelIds":[…]}`; skip internal ids (`chat_20706`, `chat_23310`, `tab_flash_lite_preview`, `tab_jump_flash_lite_preview`, `gemini-2.5-flash-thinking`, `gemini-2.5-pro`) | `cmd/fetch_antigravity_models/main.go:214-301`; `sdk/cliproxy/antigravity_models.go` | **DELTA** — a list endpoint exists; `CatalogSource::Endpoint`, probe only as fallback |
| Current pool (CLIProxyAPI `models.json`, identical to its live catalog) | `claude-opus-4-6-thinking`, `claude-sonnet-4-6`, `gemini-3-flash`, `gemini-3.1-flash-lite`, `gemini-3.1-flash-image`, `gemini-3.1-pro-low`, `gemini-pro-agent` (= Gemini 3.1 Pro High), `gemini-3.6-flash-high`, `gemini-3.7-flash-high`, `gemini-3.8-flash-high`, `gpt-oss-120b-medium`; bare `gemini-3-pro` removed 2026-07-18 | `internal/registry/models/models.json` | **DELTA** — the brief's list is stale. Presets: `fast` = `gemini-3.1-flash-lite`; `default`/`vision` = `gemini-3.8-flash-high` or `claude-sonnet-4-6` (owner's setting); `reasoning` = `claude-opus-4-6-thinking` or `gemini-pro-agent` |
| Quota model | two shared pools with rolling 5-hour and weekly windows: Gemini (Pro + Flash) and non-Gemini (Claude, GPT-OSS); `POST /v1internal:retrieveUserQuota` `{project}` → `buckets[]{remainingFraction, resetTime, modelId}` (and `retrieveUserQuotaSummary`) | `quota.ts:217-251`; openusage provider doc | `RateLimited{until}` reads `resetTime`; the HUD can show remaining fraction |
| 429 | `{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","details":[{"@type":"…RetryInfo","retryDelay":"3.9s"}, {"@type":"…ErrorInfo","reason":"RATE_LIMIT_EXCEEDED"\|"QUOTA_EXHAUSTED"\|"MODEL_CAPACITY_EXHAUSTED"\|"INSUFFICIENT_G1_CREDITS_BALANCE"}]}}`; `QuotaFailure.violations[].quotaId` containing `PerDay` = terminal for the day | `internal/runtime/helps/json_retry_helpers.go`; `credits.go:216-272` | no quota response headers known (VERIFY by capture) |
| 403 signals | `VALIDATION_REQUIRED` (with a verification link in the Help detail) · `This service has been disabled in this account for violation of Terms of Service. If you believe this is an error, contact gemini-code-assist-user-feedback@google.com.` (the ban; also "Gemini has been disabled in this account…") · `The caller does not have permission` · `…lack a Gemini Code Assist license…` | CLIProxyAPI #1637, #1729; plugin `googleQuotaErrors.ts`; gemini-cli #24962 | see the drift table — the ToS message stops every connected Google account |
| Client-version rejection | `This version of Antigravity is no longer supported` | plugin TROUBLESHOOTING; `antigravity_version.go:19-23` | = the UA version is too old → `Unavailable{FingerprintDrift}`; bump from the manifest |
| Terms and enforcement | Antigravity Additional Terms (read 2026-09-11): *"Using third party software, tools, or services to access the Service (e.g. using OpenClaw with Antigravity OAuth) is a breach of this Agreement. Such actions may be grounds for suspension or termination of your Antigravity and/or Gemini CLI accounts."* Mass 403 ToS suspensions of Pro/Ultra accounts since 2026-02; Google statement 2026-03-18 on abuse detection for "Gemini CLI oAuth with third-party software"; Gemini CLI consumer path shut down 2026-06-18 ("This client is no longer supported for Gemini Code Assist for individuals…") | `antigravity.google/terms`; gemini-cli Discussions #22970, #28017; deprecation page 2026-06-11 | owner's decision (ADR 0009); enters the consent copy verbatim and the stop-all-accounts rule |
| Capture knob | Antigravity 2.x is an Electron app: `HTTPS_PROXY` / system proxy plus a trusted local CA; the IDE fork honours VS Code's `http.proxy` | — | VERIFY on the day; the capture settles UA, host, headers, tier ids, system instruction |

## Drift signals and the extra-usage guard

| Provider | Signal | Meaning | Bluey does |
|---|---|---|---|
| any | 401 after a successful refresh attempt | token revoked / expired | `NeedsReauth`; HUD *Reconnect* |
| any | 429 with a window / reset header or body | plan limit | `RateLimited{until, window}`; role falls back until reset; copy names the window and reset time |
| ChatGPT | 403 with a policy / terms body | account or client blocked | `Unavailable{PolicyBlocked}`; one toast; fallback |
| ChatGPT | models endpoint 404 for the pinned `client_version` | endpoint moved | `Unavailable{CatalogUnavailable}`; bump `client_version` |
| Claude | HTTP 400 whose message mentions *extra usage* (`Third-party apps now draw from your extra usage, not your plan limits…`, `You're out of extra usage…`) | Anthropic no longer recognises the request as Claude Code and is billing outside the plan | **stop**: `Unavailable{FingerprintDrift \| ExtraUsageBilling}`, never retried with the same `VERSION`, one toast naming the fingerprint version and capture date, fallback |
| Claude | bare 429 `{"type":"rate_limit_error","message":"Error"}` **without** any `anthropic-ratelimit-unified-*` header | the third-party detection response — retrying unchanged never succeeds | same as above: `Unavailable{FingerprintDrift}`, stop, fallback |
| Claude | 400/429 `Extra usage is required for long context requests.` | 1M-context entitlement, not the classifier | drop `context-1m-2025-08-07` for this account, retry once; never send it by default |
| Claude | 429 with `anthropic-ratelimit-unified-{5h,7d,7d_oi}-status: rejected` and `-reset` | plan window exhausted | `RateLimited{until: <reset>, window: <representative-claim>}`; the role falls back until then; copy: "Claude 5-hour limit — resets at 14:32, using Gemini meanwhile" |
| Antigravity | 403 whose message contains *violation of Terms of Service* (`This service has been disabled in this account…` / `Gemini has been disabled in this account…`) | Google has suspended the account for third-party use | **stop every connected Google account** (`Unavailable{PolicyBlocked}` on all of them, no rotation, no retry), one toast carrying the appeal address from the message |
| Antigravity | 403 with `ErrorInfo.reason = VALIDATION_REQUIRED` and a verification link | Google wants the user to verify in a browser | `Unavailable{PolicyBlocked}` carrying the link; *Reconnect* after verifying |
| Antigravity | 403 `The caller does not have permission` / `…lack a Gemini Code Assist license…` | entitlement | `Unavailable{PolicyBlocked}` with the message text; Workspace accounts get the project-id field |
| Antigravity | `This version of Antigravity is no longer supported` | the User-Agent version is too old | `Unavailable{FingerprintDrift}`; refresh the version from the Hub manifest and re-probe |
| Antigravity | 400 `INVALID_ARGUMENT` for a request that matches the golden fixture | wrapper changed | `Unavailable{FingerprintDrift}` |
| Antigravity | 429 `RESOURCE_EXHAUSTED` — `RetryInfo.retryDelay`; `ErrorInfo.reason` `RATE_LIMIT_EXCEEDED` / `QUOTA_EXHAUSTED` / `MODEL_CAPACITY_EXHAUSTED` | pool window exhausted, or capacity | `RateLimited{until}` per account and model pool (reset time from `retrieveUserQuota`); rotate to the next connected Google account only on `QUOTA_EXHAUSTED`; capacity → one short retry, then fallback |
| Antigravity | 404 on a model id | retired / wrong pool | drop from the catalog; refresh via `fetchAvailableModels` |

The guard is a correctness requirement, not a preference: a response that says the request is
billed outside the plan halts the request before another one is sent.

## Consent copy

Shown once per provider, before the browser opens. Plain language, no legal boilerplate, and the
four facts in this order: what is sent · whose limits are used · unofficial · what Bluey does when
it stops working.

**ChatGPT**
> **Use your ChatGPT subscription with Bluey?**
> Bluey signs you in to ChatGPT in your browser and then talks to OpenAI the way the Codex CLI
> does. It sends the questions you ask and the screenshots you choose to send with ⌘↵ — nothing
> else, nothing automatically. Usage counts against your ChatGPT plan's Codex limits. This is not
> an official API: OpenAI may change or stop it without notice. If that happens, Bluey stops using
> this account, tells you, and falls back to your API key.
> *Cancel · Continue in browser*

**Claude**
> **Use your Claude subscription with Bluey?**
> Bluey signs you in to Claude in your browser and then talks to Anthropic the way Claude Code
> does. It sends the questions you ask and the screenshots you choose to send with ⌘↵. Usage
> counts against your Claude plan's limits (5-hour and weekly windows). This is not an official
> API: Anthropic's terms say these sign-in tokens are for Claude Code and its own apps only, and
> since April 2026 it bills requests it does not recognise as Claude Code to paid *Extra usage*
> instead of your plan. Bluey treats the first such response as a stop signal — it will not keep
> sending, it tells you, and it falls back to your API key.
> *Cancel · Continue in browser*

**Google AI**
> **Use your Google AI subscription with Bluey?**
> Bluey signs you in with Google in your browser and then talks to Google the way the Antigravity
> IDE does. It sends the questions you ask and the screenshots you choose to send with ⌘↵. Usage
> counts against your Google AI plan's Antigravity quota. This is not an official API, and Google's
> Antigravity terms (read 2026-09-11) say: *"Using third party software, tools, or services to
> access the Service (e.g. using OpenClaw with Antigravity OAuth) is a breach of this Agreement.
> Such actions may be grounds for suspension or termination of your Antigravity and/or Gemini CLI
> accounts."* Accounts have been suspended this way since February 2026. This is your account and
> your call. At the first sign of a block Bluey stops using every Google account you connected,
> tells you, and falls back to your Gemini API key.
> *Cancel · Continue in browser*

## Re-capture runbook

Run this after every release of the official client, whenever `accounts_probe_fingerprint`
reports drift, and before bumping a fingerprint `VERSION`.

1. **Install or upgrade the official client** and record its version (`claude --version`,
   `codex --version`, Antigravity → About).
2. **Start the capture proxy**: `bun run fingerprints:capture <provider>` — a local HTTP proxy on
   `127.0.0.1:1456` (`--port`) that forwards to the real upstream and writes every exchange,
   already scrubbed, to `tests/fixtures/fingerprints/<provider>/captures/` (git-ignored). The
   Antigravity app has no base-URL knob: run mitmproxy / Proxyman with its CA trusted
   (`HTTPS_PROXY`), export a HAR and run `bun run fingerprints:import-har antigravity <file.har>`.
3. **Point the client at it** — Claude Code: `ANTHROPIC_BASE_URL=http://127.0.0.1:1456 claude`;
   Codex CLI: `chatgpt_base_url` in `~/.codex/config.toml` (the *ChatGPT-auth* backend — VERIFY
   the exact value on the day; the proxy forwards whatever path the CLI sends); Antigravity: the
   Electron proxy setting (VERIFY on the day).
4. **Exercise it**: one text turn, one turn with an image, one turn that lists models, and (Claude)
   one turn with a tool call, so the fixture covers every shape the shaper produces.
5. **Scrubbing is automatic** (`bluey_protocols::fingerprints::scrub`): tokens, JWTs, API keys,
   account / organisation / project ids, e-mails, device and session ids, home directories and
   the user's own text and images become stable placeholders (`<ACCESS_TOKEN>`, `<UUID>`,
   `<EMAIL>`, `<TEXT n>`, `<BASE64 n>`, …) before the file is written; the fingerprint blocks
   (identity, billing header, wrapper fields, the Codex instructions template) are kept verbatim.
   Read the capture once anyway before blessing it.
6. **Diff**: `bun run fingerprints:diff <provider>` compares the newest capture header by header
   and field by field with the *documented* fingerprint (these tables as data) and with the
   blessed golden. **DRIFT** is anything the tables do not explain — a header or field added,
   removed or outside its documented format (exit 1); **VERSION** is a value that still matches
   the documented format — a client version bump — and names the column to update; **INFO** is a
   difference the rules call informational (model, optional wrappers). `--json` for tooling,
   `--endpoint <name>`, `--capture <file>`, `--against documented|golden|<file>`.
7. **Update** the shaper module and the constants in `bluey_protocols::fingerprints::<provider>`
   (`VERSION`, `CAPTURED_ON`, the documented values and rules), regenerate the documented fixtures
   (`UPDATE_FIXTURES=1 cargo test -p bluey-protocols fingerprints`), refresh the tables in this
   document (values *and* dates) and the *Status by provider* row.
8. **Bless**: `bun run fingerprints:bless <provider>` promotes the reviewed capture to
   `tests/fixtures/fingerprints/<provider>/golden/<endpoint>.json` (committed); later diffs
   compare against it as well.
9. **Probe** with the real account: `accounts_probe_fingerprint` sends a one-token request and
   reports whether it was billed to the plan (Claude: no extra-usage 400; ChatGPT: 200 with usage
   headers; Antigravity: 200 on prod).
10. **PR** with the repo's Why / What / Validation / Not verified here template; the golden fixture,
    the `VERSION` bump and this document change together.

## Importing an existing sign-in (read-only)

An alternative to a fresh browser flow on the owner's own Mac: Bluey reads the official client's
local credential store, copies the tokens into its own Keychain entry and continues exactly like
*Connect*. It never writes to the other client's store.

| Client | Location on macOS | Format | Verified |
|---|---|---|---|
| Codex CLI | `~/.codex/auth.json` (default store; `$CODEX_HOME` overrides); or Keychain service `Codex Auth`, account `cli\|<sha256(codex_home)[:16]>` when `cli_auth_credentials_store = "keyring"\|"auto"` | `{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token","access_token","refresh_token","account_id"},"last_refresh"}`; treat `last_refresh` older than 8 days as refresh-first | 2026-09-11 (`openai/codex` @ `ab95cd4`, `login/src/auth/storage.rs`) |
| Antigravity IDE (VS Code fork) | `~/Library/Application Support/Antigravity IDE/User/globalStorage/state.vscdb` (legacy: `…/Antigravity/User/globalStorage/state.vscdb`) | SQLite `ItemTable(key, value)`; key `antigravityUnifiedStateSync.oauthToken` = base64 protobuf wrapping `OAuthTokenInfo { access_token = 1; token_type = 2; refresh_token = 3; Timestamp expiry = 4 }` (legacy key `jetskiStateSync.agentManagerInitState`, field 6); the topic framing is inferred from Antigravity-Manager's writer — dump a real DB once | 2026-09-11 (Antigravity-Manager `modules/db.rs`, `utils/protobuf.rs`; openusage) — schema VERIFY |
| Antigravity 2.x / `agy` CLI | macOS Keychain generic password, service `gemini`, account `antigravity` | `go-keyring-base64:<base64 JSON {"token":{"access_token","token_type":"Bearer","refresh_token","expiry"},"auth_method":"consumer"}>` | 2026-09-11 — inferred from Antigravity-Manager's writer; `security find-generic-password -s gemini -a antigravity -w` on a signed-in Mac settles it (VERIFY) |
| gemini-cli | `~/.gemini/oauth_creds.json` | google-auth-library `Credentials` JSON | **do not import** — a different OAuth client whose consumer quota path was shut down on 2026-06-18 |
| Claude Code | macOS Keychain service `Claude Code-credentials`, account = `$(id -un)` (scope the lookup with `-a`; several entries may exist); fallback `~/.claude/.credentials.json` (or `$CLAUDE_CONFIG_DIR/.credentials.json`); `account_uuid` and e-mail from `~/.claude.json` → `oauthAccount.accountUuid` / `emailAddress` | `{"claudeAiOauth":{"accessToken":"sk-ant-oat01-…","refreshToken":"sk-ant-ort01-…","expiresAt":<ms epoch>,"scopes":[…],"subscriptionType":"pro"\|"max","rateLimitTier":"default_claude_max_5x","refreshTokenExpiresAt":<ms>}}` — copy all keys; never write back | 2026-09-11 (official `authentication.md`; griffinmartin `keychain.ts`; hermes-agent #83338) |

## Which PR enforces which invariant

| Invariant (`docs/SECURITY.md`) | Enforced by |
|---|---|
| `secrets_set` / `secrets_has` / `secrets_delete` accept only the WebView's `SECRET_KEYS` (`provider:<id>:api_key` plus the research / agent API keys); `auth:*` and `account:*` rejected at the command layer | PR 1 — `secrets::validate_webview_key` (+ tests on both layers) |
| `data_reset_all` collects failures instead of aborting (and deletes `account:*` once they exist) | PR 1 — `ResetFailures` / `reset_incomplete`; the `account:*` keys join the list in PR 2 |
| One OAuth engine (PKCE, loopback, manual paste, device code, single-flight refresh); Clerk unchanged | PR 1 — `bluey-oauth` crate (runtime) + `bluey_protocols::oauth` (pure), host-run `#[tokio::test]`s |
| Tokens Rust-only (`account:<id>:oauth_tokens`, read and written by `AccountsManager` only); WebView sees `ProviderAccount` only; the sidecar env never names account material (test) | PR 2 — `secrets::account_tokens_key`, `AccountsManager`, `agent::env_boundary_tests` |
| Redaction of `chatgpt-account-id`, `sk-ant-oat`, `sk-ant-ort`, `ya29.`, `1//` | PR 2 — `logging::redact` (+ test) |
| Loopback listener rules (127.0.0.1, one request, 8 KB, 5 s, `state` first) | PR 1 — `bluey_oauth::LoopbackListener` |
| Consent once per provider; feature flag + Cargo feature | PR 2 — `ConsentDialog` + `experimental.acceptedAccountConsents`; `experimental.subscriptionAccounts` (Settings → AI → Accounts switch); `subscription-accounts` (default on; `AccountsManager::build_enabled`) |
| A `Connecting` status never survives a restart; a failed sign-in moves the account to the status its error names (`bluey_core::accounts::status_after_error`) | PR 2 — `AccountsManager::load` / `finish_connect` |
| Fingerprint modules with `VERSION` / `CAPTURED_ON`, golden fixtures, `fingerprints:diff`, `accounts_probe_fingerprint` | PR 2b — `bluey_protocols::fingerprints::{codex, claude_code, antigravity}` (constants, rules, documented captures → `tests/fixtures/fingerprints/*/documented/`, the diff), the `bluey-fingerprints` harness (`capture`, `import-har`, `diff`, `bless`; captures scrubbed before writing); PR 3a–3c — shapers reading the same constants, real probes, blessed goldens |
| Extra-usage guard; no automatic retry of a drifted fingerprint | PR 3a — `bluey_protocols::codex::map_error` + `drift_reason` (403 policy / drift, `usage_not_included`, 400 "unsupported parameter"), `AccountsManager::note_request_error` flips the account and the router falls back (no retry); PR 3b — the Claude extra-usage 400 / bare 429; PR 3c |
| Imports read-only | PR 3a — ChatGPT reads `auth.json`, never writes, never refreshes at import; PR 3b / 3c |
| A connected account is a provider to the router only while usable; `NeedsReauth` / `Unavailable` / an active `RateLimited` window = keyless = fallback chain | PR 3a — `bluey_core::accounts::provider_config`, `AiManager::providers` |
| Catalog presets never assign a model the catalog did not return; they fill unassigned roles after a fetch and re-point stale ones | PR 3a — `bluey_core::accounts::apply_catalog_presets` |

## Open questions (brief §9) — status

| # | Question | Status on 2026-09-11 |
|---|---|---|
| 1 | Claude Code capture (CLI version, auth transport, identity text, billing header, `metadata.user_id`, betas, Stainless headers, `/v1/models` with OAuth, profile endpoint, loopback port rules) | reference implementations read; see the Claude table. Still open until the §4b.1 capture of the owner's CLI (2.1.268): the `cch` algorithm (two references disagree), the Stainless versions bundled in 2.1.268, the six-vs-five scope set, whether the token response carries `account`/`organization`, and the redirect-URI rejection reported in claude-code #93216 (open 2026-09-09) — a possible day-one login blocker. |
| 2 | OpenAI Codex (client id, authorize URL/scopes, port 1455, device-code contract, ID-token claims, headers, `instructions` validation, `client_version`, 429 headers, image limits) | built in PR 3a from the ChatGPT table. Still open until the first real sign-in + capture + probe: whether 4-scope tokens also work (Bluey sends six), whether the token response carries `expires_in` (Bluey falls back to the JWT `exp`), the `instructions` replay (template vs `""`), the 200-response usage headers the probe reads, and image byte / pixel limits |
| 3 | Antigravity (client id/secret, scopes, redirect rules, `loadCodeAssist`/`onboardUser`, wrapper/envelope, header fingerprint, system instruction, model ids per pool, sandbox hosts) | see the Google AI table |
| 4 | Import paths and formats | see *Importing an existing sign-in* |
| 5–7 | Gemini image hints, WebP, structured-output cost | `docs/LATENCY.md` |
| 8 | Rate-limit UX copy | decided with the Accounts UI in PR 2; the drift table above fixes the semantics |
