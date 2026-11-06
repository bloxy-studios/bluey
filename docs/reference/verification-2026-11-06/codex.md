# Codex OAuth — re-verified 2026-11-06 (`rust-v0.154.0` → `rust-v0.167.0`)

Brings `docs/PROVIDER_ACCOUNTS.md › ChatGPT` and `bluey_protocols::fingerprints::codex` from the
2026-09-11 pass (`docs/reference/verification-2026-09-11/codex.md`, `rust-v0.154.0`) to the
current stable Codex CLI. The pinned client version was 13 releases behind, which hid newer plan
models from the catalog (`docs/audits/BLUEY_VOICE_INTERVIEW_AUDIT_2026-11-06.md` §6).

**Method.** Source read, **not captured** from a running CLI. Every file below was fetched at both
tags with `gh api repos/openai/codex/contents/<path>?ref=<tag>` and diffed.

| Tag | Commit | Published |
|---|---|---|
| `rust-v0.167.0` (latest stable on 2026-11-06; newest prerelease `rust-v0.168.0-alpha.7`) | `086ccc833bc9cf01024ad2dd6af7dbd421b6bf9a` | 2026-11-06T01:40:00Z |
| `rust-v0.154.0` (previous pin) | `6b9826e3aa83b1a5947db50f4332cb9c65f1b340` | 2026-09-09 |

Paths are relative to `codex-rs/`; line numbers are at `rust-v0.167.0`.

## Changed, and Bluey now follows

| Fact | 0.154.0 | 0.167.0 | Source | Bluey |
|---|---|---|---|---|
| Client version | `0.154.0` | `0.167.0` — `User-Agent` `codex_cli_rs/0.167.0 (…)`, header `version: 0.167.0`, catalog `?client_version=0.167.0` | `Cargo.toml:173`; `login/src/auth/default_client.rs:152-179`; `model-provider-info/src/lib.rs:582`; `models-manager/src/lib.rs:24-31`; `codex-api/src/endpoint/models.rs:37-46` | `fingerprints::codex::{VERSION, CLIENT_VERSION, CAPTURED_ON, USER_AGENT_EXAMPLE}` |
| Instructions | top-level `instructions` (the model's template) for models without `use_responses_lite`; a leading `developer` item for the others | **no `instructions` field** in `ResponsesApiRequest`; the base instructions are always the first `input` item: `{"type":"message","role":"developer","id":"msg_<uuid v5>","content":[{"type":"input_text","text":…}]}`, skipped when empty | `codex-api/src/common.rs:281-306`; `core/src/client.rs:1075-1085,1156-1172`; `core/src/context/base_instructions.rs:42-61` | `codex::build_responses_body` puts the instructions in leading `developer` items (own prompt; or template, then own prompt); the shaper moves any stray top-level `instructions` into `input`; rule `instructions` = `Absent` |
| Loopback redirect | `http://localhost:<port>/auth/callback` | `http://127.0.0.1:<port>/auth/callback` (ports 1455, fallback 1457) | `login/src/server.rs:79-81,197` | `codex::redirect_uri` |
| Authorize query order | `response_type, client_id, redirect_uri, scope, code_challenge, code_challenge_method, id_token_add_organizations, codex_cli_simplified_flow, state, originator` (`urlencoding`, spaces `%20`) | `response_type, client_id, redirect_uri, code_challenge, code_challenge_method, state, scope, id_token_add_organizations, codex_cli_simplified_flow, originator` (`url::form_urlencoded`, spaces `+`) | `login/src/oauth/authorization.rs:21-48`; `login/src/server.rs:580-620` | `codex::authorize_url` (already form-encoded) |
| Code-exchange form order | `grant_type, code, redirect_uri, client_id, code_verifier` | `grant_type, client_id, code, redirect_uri, code_verifier` | `login/src/oauth/client.rs:79-95,115-131` | `codex::token_exchange_form` |
| Refresh body | `{client_id, grant_type, refresh_token}` | adds `refresh_reason`: `proactive_expiry` (JWT `exp` ≤ now + 5 min), `scheduled` (`last_refresh` > 8 days), `unauthorized_recovery` (after a 401), `forced`; keys serialised sorted | `login/src/oauth/client.rs:47-63,97-113,132-134`; `login/src/auth/manager.rs:221-222,1774-1796,2170-2179,3528-3546` | `codex::refresh_body(token, reason)`; `codex::refresh_reason_for(expires_at, now)` — Bluey refreshes only ahead of expiry (60 s leeway) or after a 401 |
| Plans | … `pro` "Pro", `prolite` "Pro Lite" | adds `promax`, `sci`, `sci_plus`; labels `prolite` "Pro", `pro` "Pro (More)", `promax` "Pro (Max)", `sci` "Academic Researchers", `sci_plus` "Academic Researchers Plus" | `protocol/src/auth.rs:76-97,104-134,135-158` | `codex::plan_label` |

## Changed, documented, not sent by Bluey

| Fact | 0.167.0 | Source | Why not |
|---|---|---|---|
| Workspace routing | after routing discovery for a workspace, the CLI swaps the backend origin and sends `x-openai-account-routing-override: us\|us_cr` | `model-provider/src/workspace_routing.rs:12,40-77` | discovery is for routed (data-residency) workspaces; rule added as `Optional` so a capture from such an account is not drift |
| Guardian | `x-codex-guardian` on Guardian reviewer turns (which also drop `service_tier`) | `core/src/client.rs:1984-1986,2590-2594` | Bluey has no reviewer sub-agent |
| Body field order | `model, stream, service_tier` first so gateways can route before reading `input` | `codex-api/src/common.rs:281-287` | Bluey's `serde_json` has no `preserve_order`; its keys are sorted (they never matched the CLI's order) |
| `client_metadata` | new optional keys `mcp_attribution` and, in `x-codex-turn-metadata`, `history_initialization` / `analytics_enabled` | `core/src/responses_metadata.rs:338-388,420-466` | Bluey sends no `client_metadata` (gap since 0.154.0, below) |
| `auth.json` | optional `oauth_provenance {issuer, client_id}`, `login_owner_id` | `login/src/token_data.rs:24-31` | Bluey's import reads `tokens` only and never writes back |
| FedRAMP | `X-OpenAI-Fedramp: true` now also for FedRAMP **API keys** | `model-provider/src/bearer_auth_provider.rs:16-23` | Bluey's ChatGPT path is OAuth; the ID-token claim still decides |
| Error classes | `api_bridge` refactored around retry-after and new client-side classes (content filter, policy capacity, model not found, flex unavailable, invalid audio) | `codex-api/src/api_bridge.rs` | client-side naming; status codes Bluey maps were not revisited |

## Unchanged (re-read at 0.167.0)

| Fact | Source |
|---|---|
| `originator: codex_cli_rs`; UA format `{originator}/{version} ({os} {os_version}; {arch}) {terminal}` [` ({suffix})`]; default headers `originator`, `User-Agent`, optional `x-openai-internal-codex-residency` | `login/src/auth/default_client.rs:42,152-179,461-474` |
| Base URL `https://chatgpt.com/backend-api/codex`; built-in provider headers `version` + env `OpenAI-Organization` / `OpenAI-Project` | `model-provider-info/src/lib.rs:86,568-613` |
| `Authorization: Bearer …`, `chatgpt-account-id` (constant moved to `http-client`) | `model-provider/src/bearer_auth_provider.rs:35-51`; `http-client/src/openai_headers.rs:3` |
| `session-id`, `thread-id` (file identical) | `codex-api/src/requests/headers.rs:5-14` |
| `x-codex-beta-features`, `x-codex-turn-state`; `OpenAI-Beta` only on the WebSocket handshake | `core/src/client.rs:1512-1547,2602-2629` |
| `/models` request: `{base}/models?client_version=<major.minor.patch>`; originator override only for CCA originators (`codex_work_*`, `chatgpt_cca`) | `codex-api/src/endpoint/models.rs:37-46`; `model-provider/src/models_endpoint.rs:217-225,299-318`; `models-manager/src/originator.rs` |
| `ModelInfo` fields Bluey reads (slug, display_name, description, default_reasoning_level, supported_reasoning_levels, visibility, priority, additional_speed_tiers, service_tiers, context_window, input_modalities, model_messages.instructions_template); new optional `available_access_programs`, `supports_reasoning_effort_updates` | `protocol/src/openai_models.rs:414-516,579-597` |
| `ServiceTier::Fast` = `"priority"` (`"fast"` alias), `Flex` = `"flex"`, `"default"` dropped; a tier is sent only when the model lists it (flex excepted) | `protocol/src/config_types.rs:528-552`; `protocol/src/openai_models.rs:236-240,1116-1130`; `core/src/client.rs:1139-1149` |
| `include: ["reasoning.encrypted_content"]`, no `stream_options` by default (`output_token_replay`, `concurrent_reasoning_summaries` are under development, off) | `core/src/client.rs:1113-1122`; `features/src/lib.rs:1953-1962` |
| Client id, token / revoke URLs, refresh window (5 min) and interval (8 days), permanent refresh failures | `login/src/auth/manager.rs:221-231,1808-1841,1863` |
| Scopes (six), PKCE (64 bytes, S256 — moved to `login/src/oauth/pkce.rs`), `state` (32 bytes) | `login/src/server.rs:612-614`; `login/src/oauth/pkce.rs`; `login/src/oauth/authorization.rs:44-48` |
| Device flow endpoints and bodies, `deviceauth/callback` redirect | `login/src/device_code_auth.rs:68,107,175,206` |
| ID-token claims | `login/src/token_data.rs` (additions only) |
| SSE event names (same set) | `codex-api/src/sse/responses.rs` |
| Rate-limit headers (file identical) | `codex-api/src/rate_limits.rs` |

## Catalog snapshot

The bundled catalog (`models-manager/models.json`) — a snapshot; the backend answers `/models`
per account and leaves out models whose `minimal_client_version` is newer than `client_version`.

| Slug | Visibility | Priority | `minimal_client_version` | Service tiers / default | `use_responses_lite` |
|---|---|---|---|---|---|
| `gpt-6.1-sol` | list | 1 | 0.153.0 | priority | yes |
| `gpt-6-astra` | list | 2 | 0.153.0 | priority | yes |
| `gpt-6-sol` | list | 3 | **0.155.0** | priority / **priority** | yes |
| `gpt-6-luna` | list | 4 | **0.155.0** | priority / **priority** | yes |
| `gpt-5.6-sol` | list | 5 | 0.144.0 | priority (0.154.0 also `ultrafast`) | yes |
| `gpt-6.1-terra` | list | 7 | 0.153.0 | priority | yes |
| `gpt-5.6-terra` | list | 8 | 0.144.0 | priority | yes |
| `gpt-5.6-luna` | list | 9 | 0.144.0 | priority | yes |
| `gpt-5.5` | list | 13 | 0.124.0 | priority | no |
| `gpt-daybreak-blue-latest`, `gpt-daybreak-red-latest`, `codex-auto-review` | hide | 11, 12, 43 | — | — | yes |

New since the 0.154.0 bundle: `gpt-6.1-sol`, `gpt-6.1-terra`, `gpt-6-sol`, `gpt-6-luna`; gone:
`gpt-5.4`, `gpt-5.4-mini`, `gpt-5.2`. With `client_version=0.154.0` the backend would leave out
`gpt-6-sol` and `gpt-6-luna`. Effort values now include `minimal`, `max`, `ultra`, `persistent`
and model-defined ones (`protocol/src/openai_models.rs:68-81`); `available_in_plans` newly names
`promax`, `ent26`, `law`, `hc_consumption`. The CLI never reads `available_in_plans` (no source
file references it) — Bluey's soft plan filter is Bluey's own.

## Known gaps (already present at 0.154.0, not mirrored)

1. **Responses-lite.** For models with `use_responses_lite` (every listed model except `gpt-5.5`)
   the CLI sends `x-openai-internal-codex-responses-lite: true` (`core/src/client.rs:183,2622-2629`),
   `reasoning.context: "all_turns"` (`:1000-1021`), `parallel_tool_calls: false` (`:1161`), and no
   `tools` field — tools travel as an `additional_tools` developer item only when there are any
   (`:1059-1074`). Bluey sends `tools: []` and neither the header nor the context. Following it
   needs `use_responses_lite` from the catalog in Bluey's model capabilities.
2. **Compatibility metadata.** The CLI sends `client_metadata` (installation, session, thread, window
   and turn ids; `core/src/responses_metadata.rs:338-388`) and the headers `x-codex-window-id`,
   `x-codex-turn-metadata` (`:390-…`, `core/src/client.rs:936-950`). Bluey sends none of them.
3. **Item ids.** The base-instructions item carries `id: msg_<uuid v5 of (uuid v5 of the thread id
   in the OID namespace), text>` (`core/src/client.rs:1075-1084`; `protocol/src/response_item_id.rs:24-26`).
   Bluey leaves it out (ids are optional on input items).
4. **Field order** — see above.
