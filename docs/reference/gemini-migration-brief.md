# Bluey → Gemini-default migration — implementation brief for a coding agent

You are working in the repository `bloxy-studios/bluey` (Tauri v2 macOS "real-time AI desktop copilot"). Your job is to make **Google Gemini, driven by ONE Google AI Studio API key, the default provider for every AI feature** — chat answers, screenshot vision, live + batch transcription, embeddings, and deep research — while keeping **Microsoft Foundry / Azure OpenAI, Anthropic and OpenAI-compatible** providers as alternates that can be switched on with one env var or one click in Settings. You must also finish wiring the app so the frontend is **fully built and running against a real backend**, not only the mock transport.

Work in small, reviewable PRs (series below). Validate locally before every push. Greptile is installed on the `bloxy-studios` org: read its review comments and iterate until the score is 5/5 before merging. Never commit secrets. Bun only (`bun install`, `bun run`, `bunx`, `bun test`) — never npm/npx/yarn/pnpm.

---

## 0. Ground truth you must verify on checkout (this brief was written against commit `ac007b6`)

Run `git rev-parse --short HEAD` and re-check each item; if anything below is no longer true, adapt rather than duplicate work.

1. **The Rust app crate is a shell.** `src-tauri/src/lib.rs` is the 14-line Tauri template (`greet` only, no `mod` declarations). The modules that *do* exist under `src-tauri/src/` (`ai/`, `sidecar/`, `settings/`, `secrets/`, `sessions/`, `modes/`, `state/`, `storage/`, `events/`, `logging/`, `capture/`, `accessibility/`) are never compiled, and `src-tauri/src/state/mod.rs` (`AppCore`, lines ~166-189) references managers that do **not exist**: `crate::audio::AudioManager`, `crate::agent::AgentManager`, `crate::research::ResearchManager`, `crate::documents::DocumentsManager`, `crate::auth::AuthManager`, `crate::overlay::PanelManager`, `crate::shortcuts::ShortcutManager`, `crate::permissions::PermissionManager`, plus `crate::platform` and `crate::app` (incl. `app::set_log_level`). There is no `commands` module. `tests/integration/command-surface.test.ts` (parses `generate_handler![...]` in `lib.rs` against `COMMAND_NAMES` in `src/lib/tauri/commands.ts`, ~120 commands) therefore fails today. **Every UI path currently runs only against `src/lib/tauri/mock/`.**
2. **The documented `.env` import does not exist.** `.env.example` and `docs/SECURITY.md` promise that `AZURE_FOUNDRY_*` / `BLUEY_MODEL_*` are read at startup and moved into the Keychain; no Rust code does this. `secrets::load_dotenv()` exists but is never called. `AiSettings::default().providers` is empty.
3. **Cloud transcription is codec-only.** `bluey-protocols::{voice_live, realtime}` exist, but `docs/AI_ARCHITECTURE.md` admits "The WebSocket manager is not wired yet" and there is no Rust `audio` module at all. Only the Swift helper's on-device Apple Speech path is real.
4. **Deep research is Claude-only** (`sidecars/agent`, Claude Agent SDK, ~250 MB embedded CLI per arch) and nothing in Rust spawns it.
5. **The provider layer that does exist is good and should be extended, not rewritten**: `AiProvider` trait (`stream` / `embed` / `list_models`) in `src-tauri/src/ai/providers/mod.rs`, `build_provider` keyed on `bluey_core::types::AiProviderKind { AzureFoundry, Anthropic, OpenaiCompatible, Mock }`, role-based routing in `bluey-core/src/router.rs` over `ModelRoleAssignments { default, fast, reasoning, vision, research, transcription, embedding }`, pure wire codecs in the dependency-free `bluey-protocols` crate, SSE consumption via `eventsource-stream`, cancellation via `CancellationToken`, secrets via `keyring` (`provider:<id>:api_key`).
6. **Frontend**: React 19 + TS + Tailwind v4 + Zustand; HUD / Settings / Onboarding windows are well built and tested against the mock, but the provider UI knows only `azure_foundry | anthropic | openai_compatible`, onboarding has no API-key step, and several documented features have no producer (details in §6).

Companion documents live in `docs/reference/` — read them before starting: `gemini-api-sept-2026.md` (verified Gemini API/SDK reference with verbatim samples and REST shapes, dated 2026-09-08; §1 below reproduces the decision-relevant facts, the reference has the full request/response shapes and sources) and `bluey-sidecar-and-frontend-audit.md` (file/line-level audit of the sidecar and frontend that §3 PR 5–7 build on). This brief itself is committed as `docs/reference/gemini-migration-brief.md`.

---

## 1. Verified Gemini facts to code against (checked on ai.google.dev on 2026-09-08 — do NOT substitute training-data knowledge)

### 1.1 Models per role (pin stable IDs; never use `-latest` aliases in production)

| Bluey role | Model ID | Why | Limits / price (standard tier) |
|---|---|---|---|
| `default`, `vision`, `research` | `gemini-3.8-flash` | GA 2026-09-02, most capable Flash; text+image+audio+video+PDF in; structured output, function calling | 1,048,576 in / 65,536 out; $0.75/$3.75 per 1M until 2026-12-31, then $1.50/$7.50 |
| `fast` (classification, quick replies, summarization) | `gemini-3.5-flash-lite` | cheapest 3.5-gen, supports `thinkingLevel: minimal` | $0.30/$2.50 |
| `reasoning` | `gemini-3.8-flash` with `thinkingLevel: high` (opt-in alternative: `gemini-3.1-pro-preview` — **no free tier**, $2/$12) | | |
| `transcription` (live) | `gemini-3.5-transcribe-live` | GA Aug 2026; Live API WebSocket only; interim + final events | **10-minute session cap**; PCM16 LE 16 kHz mono; no diarization/word timestamps on live; ~$0.009/min |
| transcription (batch/file) | `gemini-3.5-transcribe` | `generateContent` with `audioTranscriptionConfig` | ≤1 h/request (≤30 min with diarization or word timestamps); diarization ≤8 speakers; ~$0.005/min |
| `embedding` | `gemini-embedding-2` | GA 2026-04-22; 3072 dims default, MRL-truncate to 768/1536 (auto-normalized) | 8,192 input tokens; $0.20/1M; **no `taskType`** — use prompt prefixes; **not** compatible with `gemini-embedding-001` vectors |

Do **not** use: `@google/generative-ai` (deprecated 2025-11-30), any `gemini-2.0-*` (shut down), `gemini-embedding-2-preview` (shut down 2026-08-10 but still in some docs), `gemini-3-pro-preview` (redirects), `gemini-flash-latest` (hot-swapped alias). `gemini-2.5-flash` still works but is more expensive than 3.5-flash-lite for less capability.

### 1.2 Endpoints, auth, SDK

- REST base: `https://generativelanguage.googleapis.com/v1beta`. Auth header: `x-goog-api-key: <key>` (never `?key=` on HTTP; the Live WebSocket URL is the one documented place the key goes in the query string).
- `POST /models/{model}:generateContent` (unary) and `POST /models/{model}:streamGenerateContent?alt=sse` (SSE; each `data:` line is a full `GenerateContentResponse` chunk). `POST /models/{model}:embedContent`, `POST /models/{model}:batchEmbedContents`, `GET /models?pageSize=…&pageToken=…`.
- Live API WebSocket: `wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent?key=<key>`. First frame must be `setup`; wait for `setupComplete` before sending audio.
- JS SDK (sidecar only): `@google/genai` **2.21.0** (2026-09-02). Pin `"@google/genai": "^2.21.0 <3"` — v3 requires Node 22 and removes automatic function calling from `generateContent`. `import { GoogleGenAI, ThinkingLevel, Type, Modality } from "@google/genai"`; `new GoogleGenAI({ apiKey })`; `ai.models.generateContentStream({ model, contents, config })`; `config.abortSignal`, `config.httpOptions.{timeout, retryOptions}`; errors are `ApiError` with `.status`. Bun compatibility of the SDK is **undocumented** — see §5.4 for the decision rule.
- API keys: create a **new** key in Google AI Studio. New keys are "auth keys"; Google states that unrestricted standard keys are already rejected and that all standard keys are rejected in September 2026. The key must never reach the WebView bundle or the frontend process — it lives in the macOS Keychain and is used only by Rust and by the Bun sidecar's environment.
- Rate limits: Google no longer publishes per-model free-tier numbers; third-party measurements put the free tier at roughly 5 RPM / 20 RPD for the Flash line (unverified). 429 `RESOURCE_EXHAUSTED` must be handled gracefully (see 1.6). For real testing, link billing (Tier 1) in AI Studio.

### 1.3 generateContent request/response shapes (camelCase; this is what the Rust adapter speaks)

Request body:
```json
{
  "systemInstruction": { "parts": [ { "text": "<all AiRole::System messages joined>" } ] },
  "contents": [
    { "role": "user",  "parts": [ { "text": "..." } ] },
    { "role": "model", "parts": [ { "text": "..." } ] },
    { "role": "user",  "parts": [
        { "inlineData": { "mimeType": "image/jpeg", "data": "<base64>" } },
        { "text": "What is on this screen?" }
    ] }
  ],
  "generationConfig": {
    "maxOutputTokens": 1200,
    "thinkingConfig": { "thinkingLevel": "low" },
    "responseMimeType": "application/json",
    "responseJsonSchema": { "...": "JSON Schema (see 1.5)" }
  }
}
```
Rules: roles are exactly `user` and `model` (map `assistant` → `model`; fold every `system` message into `systemInstruction`). Parts: `text`, `inlineData{mimeType,data}`, `fileData{fileUri,mimeType}`, `functionCall{id,name,args}`, `functionResponse{id,name,response}`; model parts may carry `"thought": true` (summary text — do not display as answer) and `"thoughtSignature": "<base64>"` (echo back verbatim if you ever resend model turns; Bluey's `AiRequest.messages` are rebuilt per request from TS, so the Rust adapter does not need to persist signatures — but it must not drop them from a `contents` array it forwards).

**Gemini 3.x rules:** never send `temperature`, `topP`, `topK` (deprecated 2026-07-21; Google says leave the default 1.0 — lower values can loop), never `candidateCount`, never `thinkingBudget` together with `thinkingLevel` (400). `thinkingLevel` values: `low | medium (default) | high`; `minimal` **errors on 3.7/3.8** but works on 3.6/3.5/3.5-flash-lite. Thinking cannot be turned off on Gemini 3; thinking tokens are billed as output. Images default to 1,120 tokens each (= `high` resolution) — fine for 1600-px screenshots. Inline request size ≤ 20 MB (image guide); Files API (48 h retention, 2 GB/file) only for large audio.

Response chunk (SSE) / unary response:
```json
{
  "candidates": [ { "content": { "parts": [ { "text": "..." } ], "role": "model" }, "finishReason": "STOP", "index": 0 } ],
  "usageMetadata": { "promptTokenCount": 4, "candidatesTokenCount": 12, "thoughtsTokenCount": 30, "totalTokenCount": 46 },
  "modelVersion": "gemini-3.8-flash", "responseId": "..."
}
```
Concatenate `candidates[0].content.parts[*].text` for parts without `thought: true`. A response may carry `promptFeedback.blockReason` and no candidates → treat as an error. `finishReason`: `STOP` → `FinishReason::Stop`; `MAX_TOKENS` → `Length`; `SAFETY`, `RECITATION`, `PROHIBITED_CONTENT`, `BLOCKLIST`, `SPII`, `MALFORMED_FUNCTION_CALL`, `OTHER` → `FinishReason::Error` with `BlueyError::ai("blocked_<lowercase reason>", …)`. Output tokens for metrics = `candidatesTokenCount + thoughtsTokenCount`.

Error body (map by `status`, never log or surface `message` verbatim — it can echo prompts; the codebase already forbids including bodies):
```json
{ "error": { "code": 429, "message": "...", "status": "RESOURCE_EXHAUSTED",
  "details": [ { "@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "23s" },
               { "@type": "type.googleapis.com/google.rpc.QuotaFailure", "violations": [ { "quotaId": "GenerateRequestsPerDayPerProjectPerModel-FreeTier", "quotaValue": "20" } ] },
               { "@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": "API_KEY_INVALID" } ] } }
```

### 1.4 Embeddings shapes
```json
POST /models/gemini-embedding-2:batchEmbedContents
{ "requests": [ { "model": "models/gemini-embedding-2", "content": { "parts": [ { "text": "title: none | text: <chunk>" } ] }, "outputDimensionality": 768 }, ... ] }
→ { "embeddings": [ { "values": [ ... ] }, ... ] }      (single embedContent → { "embedding": { "values": [...] } })
```
Prefix convention for `gemini-embedding-2` (text only; be consistent between indexing and querying): documents `title: {title or none} | text: {content}`; queries `task: search result | query: {question}`. Multiple parts inside ONE `content` collapse to ONE vector — one `requests[]` entry per chunk. Response key names (`embedding`/`embeddings`) are the long-standing REST shape; assert them in a test against a recorded fixture and verify once against the live API.

### 1.5 Structured output
Use `generationConfig.responseMimeType: "application/json"` + `generationConfig.responseJsonSchema: <schema>` (`responseSchema` is deprecated; the newest docs also show `responseFormat: { text: { mimeType, schema } }` — either is acceptable, pick `responseJsonSchema` and add a unit test). Supported subset: `type` (incl. type arrays for null), `title`, `description`, `properties`, `required`, `additionalProperties`, `enum`, `format` (date-time/date/time), `minimum/maximum`, `items`, `prefixItems`, `minItems/maxItems`, `$ref/$defs/$id/$anchor`, `anyOf` (`oneOf` treated as `anyOf`), non-standard `propertyOrdering`. Bluey's schemas come from zod v4 `z.toJSONSchema()` (`src/modes/schemas.ts:137`) — strip the top-level `$schema` key before sending. Streaming chunks are valid partial JSON to concatenate, which matches the existing `extractPartialStringField(accumulated, "content")` draft logic in `src/ai/engine.ts`.

### 1.6 Error → `BlueyError` mapping (extend `map_http_status` in `providers/mod.rs` with a Gemini-aware variant that parses `error.status` / `details[].reason`)

| HTTP / status | Reason (details) | BlueyError kind / code | Recovery |
|---|---|---|---|
| 400 `INVALID_ARGUMENT` | `API_KEY_INVALID` | configuration `config.api_key_invalid` | ConfigureProvider |
| 400 `INVALID_ARGUMENT` | other | ai `ai.invalid_request` (mention thinking/sampling params if the request had them) | — |
| 403 `PERMISSION_DENIED` | — | configuration `config.http_403` | ConfigureProvider |
| 404 `NOT_FOUND` | model | configuration `config.model_not_found` ("pick another model") | ConfigureProvider |
| 429 `RESOURCE_EXHAUSTED` | RetryInfo.retryDelay / QuotaFailure.quotaId | network `network.http_429`, include `retry_after_ms` and whether `quotaId` contains `PerDay` (daily quota → message "daily free-tier quota reached; wait until midnight Pacific or enable billing") | Retry |
| 500/503/504 `INTERNAL`/`UNAVAILABLE`/`DEADLINE_EXCEEDED` | — | network `network.http_5xx` | Retry |
| blocked (`promptFeedback.blockReason`, finish `SAFETY`…) | — | ai `ai.blocked_<reason>` | — |

Retry policy (non-streaming calls: embed, list_models, test): up to 3 attempts on 429/5xx with exponential backoff + jitter, honouring `retryDelay` when present; never retry 400/403/404. Streaming: retry only if no byte has been received yet.

### 1.7 Live transcription WebSocket protocol (for `gemini-3.5-transcribe-live`)

Client → server frames (exactly one top-level key per JSON message):
```json
{ "setup": { "model": "models/gemini-3.5-transcribe-live",
             "generationConfig": { "responseModalities": ["TEXT"] },
             "inputAudioTranscription": { "languageCodes": [], "customVocabulary": [] },
             "realtimeInputConfig": { "automaticActivityDetection": { "disabled": false, "silenceDurationMs": 800 } } } }
{ "realtimeInput": { "audio": { "data": "<base64 PCM16 LE 16 kHz mono>", "mimeType": "audio/pcm;rate=16000" } } }
{ "realtimeInput": { "audioStreamEnd": true } }
```
Server → client: `setupComplete`; `serverContent.interimInputTranscription.text` (speculative, frequent — replaces the current partial), `serverContent.inputTranscription{text, languageCode}` (final, authoritative), `goAway.timeLeft` (reconnect cue), `sessionResumptionUpdate{newHandle, resumable}` (whether `sessionResumption` applies to the transcribe model is **unverified** — test it; do not depend on it), `error`. Constraints: audio chunks ~100 ms (Google's guidance; the helper's 200 ms chunks also work but set `chunkMs: 100` for the Gemini path), `languageCodes: []` = auto-detect, `mode: "SMART"` and `customVocabulary` (≤1,000 terms) are supported on live, diarization/word timestamps are **not**. Hard **10-minute session limit** → rotate sessions (see §4.4). Hybrid VAD: keep server VAD on and additionally send `audioStreamEnd: true` when the helper's VAD reports end of speech, which forces immediate finalization.

### 1.8 Function calling (sidecar research loop)
`config.tools = [{ functionDeclarations: [{ name, description, parameters | parametersJsonSchema }] }]`; the model returns parts `{ functionCall: { id, name, args } }`; push `response.candidates[0].content` back **unchanged** (it carries `thoughtSignature`), then a `user` turn with `{ functionResponse: { id, name, response: { result } } }` whose `id` **must equal** the call's `id` (mismatched id/name/count on 3.x yields an empty `STOP` response). Structured output may be combined with function calling on Gemini 3, but the robust pattern is: tool loop in text mode, then one final schema-constrained "write the report" turn without tools.

---

## 2. Target design — "Gemini by default, one key, switchable"

### 2.1 Provider model
- Add `AiProviderKind::GoogleGemini` (serde `google_gemini`) in Rust and `"google_gemini"` in `src/lib/types/ai.ts`. Reserved provider ids created by bootstrap: `gemini`, `azure-foundry`, `anthropic`, `openai`. User-created providers keep `createId("provider")` ids.
- `AiProviderConfig.baseUrl` becomes optional for Gemini (default `https://generativelanguage.googleapis.com/v1beta`; `GEMINI_BASE_URL` may override for proxies). `apiVersion`/`deployments` are Azure-only fields — hide them for other kinds in the UI, ignore them in the Gemini adapter.
- **Presets** (new pure module `bluey_core::presets`): a table `kind → { default, fast, reasoning, vision, research, transcription?, embedding?, thinking policy, embedding_dims }` for every kind: Gemini (table in §1.1), AzureFoundry (`gpt-5.6-terra`, `gpt-5.6-luna`, `gpt-6-astra`, `gpt-6-astra`, `claude-sonnet-5` via Foundry or `gpt-5.6-terra`, `MAI-Transcribe-1.5`, `text-embedding-3-small`), Anthropic (`claude-sonnet-5`, `claude-haiku-4-5`, `claude-opus-5`, `claude-sonnet-5`, `claude-sonnet-5`, none, none), OpenaiCompatible (leave models empty — user must pick). Roles a provider cannot serve (Anthropic embeddings/transcription) stay `null`, and the router's existing fallbacks apply; when another configured provider can serve them, presets may point those roles at it (e.g. Anthropic default + Gemini embeddings/transcription).
- **The switch has two entry points that converge on `ModelRoleAssignments` + `audio.transcriptionProvider` + `RESEARCH_BACKEND`:**
  1. **Env at bootstrap** (`BLUEY_AI_PROVIDER`, see 2.2) → `app::env_import` seeds providers, moves keys to the Keychain, applies presets.
  2. **Settings → AI → "Default provider"** select + "Use recommended models" → new command `ai_apply_provider_presets { providerId, overwrite: boolean }` → Rust applies `presets::for_kind(kind)` (only filling `null` roles unless `overwrite`), sets `audio.transcriptionProvider` (`gemini_live` for Gemini, `cloud_realtime` for Azure, `apple` otherwise), sets `ai.researchBackend` (`gemini` when the default provider is Gemini, `claude` when Anthropic/Foundry-Claude is available, else `gemini` if a Gemini key exists), persists, publishes `settings.changed`.

### 2.2 Environment contract (rewrite `.env.example`; keep the Clerk block; keep existing Azure/Anthropic blocks but make them clearly optional)
```
# ── Provider switch ───────────────────────────────────────────────────────────
# Which configured provider owns the model roles after bootstrap. Change it and restart,
# or switch at runtime in Settings → AI → Default provider. Keys for ALL providers may be
# present at once; only the selected one is used by default.
BLUEY_AI_PROVIDER=gemini            # gemini | azure_foundry | anthropic | openai_compatible

# ── Google Gemini (default) — one Google AI Studio key serves chat, vision, live + file
#    transcription, embeddings and deep research. Create a NEW key at
#    https://aistudio.google.com/apikey (standard keys are rejected from Sept 2026).
GEMINI_API_KEY=                     # alias: GOOGLE_API_KEY (GEMINI_API_KEY wins)
GEMINI_BASE_URL=                    # optional; default https://generativelanguage.googleapis.com/v1beta
BLUEY_EMBEDDING_DIMENSIONS=768      # 768 | 1536 | 3072 (gemini-embedding-2 auto-normalizes truncated vectors)
BLUEY_TRANSCRIPTION_PROVIDER=gemini_live   # apple | gemini_live | cloud_realtime — default gemini_live when a Gemini key exists
RESEARCH_BACKEND=gemini             # gemini | claude — deep-research agent backend

# Optional per-role overrides for the ACTIVE provider (defaults come from built-in presets):
BLUEY_MODEL_DEFAULT=gemini-3.8-flash
BLUEY_MODEL_FAST=gemini-3.5-flash-lite
BLUEY_MODEL_REASONING=gemini-3.8-flash
BLUEY_MODEL_VISION=gemini-3.8-flash
BLUEY_MODEL_RESEARCH=gemini-3.8-flash
BLUEY_MODEL_TRANSCRIPTION=gemini-3.5-transcribe-live
BLUEY_MODEL_EMBEDDING=gemini-embedding-2

# ── Alternates (leave empty unless you have credits) ─────────────────────────
# Microsoft Foundry / Azure OpenAI … (existing block, unchanged semantics)
# Anthropic / Claude on Foundry … (existing block; ANTHROPIC_API_KEY also feeds the Claude research backend)
# OpenAI-compatible
OPENAI_API_KEY=
OPENAI_BASE_URL=https://api.openai.com/v1
# Research tools, logging … (existing)
```
Precedence: env → Keychain import happens only when the Keychain has no key for that provider (so keys typed in Settings are never clobbered), unless `BLUEY_ENV_OVERRIDES_KEYCHAIN=1`. `BLUEY_MODEL_*` overrides are applied on top of presets for the active provider only. Persist `ai.bootstrapProvider` (new settings field) so a changed `BLUEY_AI_PROVIDER` re-applies presets on the next boot while an unchanged one leaves user edits alone. Log only "imported api key for provider gemini" — never values, never key lengths.

### 2.3 Thinking-level policy for Gemini (pure function in the codec, unit-tested)
| Request | thinkingLevel |
|---|---|
| `task=classification`, `latency=ultra-fast`, `fast` role on 3.5-flash-lite | `minimal` (only when `supports_minimal(model)`, i.e. not 3.7/3.8) else `low` |
| `task=answer` with `latency=fast` | `low` |
| `latency=balanced`, `task=coding`, `task=summarization` | `medium` (omit the field — it is the default) |
| `reasoning=deep`, `task=deep_reasoning`, `task=system_design` with reasoning ≥ light, `latency=deep` | `high` |

The Gemini adapter **ignores** `ProviderRequest.temperature` for any model id matching `^gemini-3` and forwards it only for `gemini-2.5-*`. Do not change `src/ai/request.ts::temperatureFor` (other providers still use it).

### 2.4 Transcription provider model
`TranscriptionProviderKind` gains `GeminiLive` (serde `gemini_live`) in Rust (`bluey-core/src/types/transcript.rs`) and `"gemini_live"` in TS (`src/lib/types/transcript.ts`). Semantics: `apple` = on-device Apple Speech (works offline, no key); `gemini_live` = Gemini Live API STT via Rust WebSocket (default when a Gemini key exists); `cloud_realtime` = existing Voice Live / OpenAI realtime codecs (wire them into the same `TranscriptionProvider` trait so the Azure alternate also works). If the selected cloud provider has no usable key or the network is down, the audio session must still start with `apple` and surface a non-fatal `audio.error{code: "stt_fallback"}`.

### 2.5 Research backend model
`sidecars/agent` gains `RESEARCH_BACKEND=gemini|claude`. Rust's new `AgentManager` passes exactly one backend's credentials into the sidecar env: for `gemini`, `GEMINI_API_KEY` (from Keychain `provider:<gemini provider id>:api_key`) plus `BLUEY_RESEARCH_MODEL` (the `research` role model); for `claude`, the existing `ANTHROPIC_*` / `CLAUDE_CODE_USE_FOUNDRY` set. The wire protocol (`docs/AGENT_SIDECAR_PROTOCOL.md`) does not change.

---

## 3. PR series (do them in order; each PR must pass `bun run typecheck && bun run lint && bun run test && bun run build` and `bun run check:rust --darwin` on Linux/macOS, plus `bun run tauri:dev` smoke on macOS where noted)

### PR 0 — Wire the Rust app crate (prerequisite for anything real)
Goal: `src-tauri/src/lib.rs` declares every module, builds `AppCore`, registers plugins, and registers **every** name in `COMMAND_NAMES` (`src/lib/tauri/commands.ts`) in `generate_handler!`, so `tests/integration/command-surface.test.ts` passes and `bun run check:rust --darwin` type-checks the app crate.
- Create the missing modules with the APIs their call sites already expect: `audio` (`AudioManager` — full implementation lands in PR 4; here a compilable manager that drives the helper's `audio.*` methods and re-emits `audio.*`/`transcript.*` events for the Apple path), `agent` (`AgentManager`: spawn `bluey-agent` per job via `tauri_plugin_shell` sidecar, JSON-Lines over stdin/stdout mirroring `sidecar::HelperClient`, map events with `bluey_protocols::agent`, answer `document.request` from `bluey-storage`, cancel + kill on app exit), `research` (`ResearchManager`: Exa `POST /search` and Firecrawl `POST /v2/scrape` with `bluey_protocols::{exa,firecrawl}`, `research_available`), `documents` (`DocumentsManager`: add/list/get/text/delete/retrieve/reindex/pick_files over `bluey_storage::documents`, calling `AiManager::embed` when embeddings are enabled), `auth` (`AuthManager`: Clerk status/token cache per ADR 0003; `auth_fapi_fetch` proxy), `overlay` (`PanelManager` over `tauri-nspanel`: show/hide/toggle/move/resize/opacity/pinned/expanded/drag, `panel.*` events, content protection via `capture_set_protection`), `shortcuts` (`ShortcutManager` over `tauri-plugin-global-shortcut` + `bluey_core::shortcuts`: register/update/reset/conflict check, `shortcut.triggered` events), `permissions` (`PermissionManager`: screen recording / microphone / accessibility / notifications status + request + open settings, via helper `permissions.*` and `core-graphics`/`accessibility-sys`), `platform` (tray/menu bar item, autostart, window open/close, `data_*` commands), `app` (bootstrap sequence, `set_log_level`, `env_import` stub that only calls `load_dotenv()` in this PR, `app_*` commands, `dev_*` commands).
- Derive exact signatures from `src/lib/tauri/commands.ts` (parameter/return types mirror `src/lib/types/*` ⇄ `bluey_core::types`), `src/lib/tauri/events.ts` ⇄ `bluey_core::events`, `docs/ARCHITECTURE.md`, `docs/HELPER_PROTOCOL.md`, ADRs 0001–0006, and the existing managers' `new/load` constructors.
- Bootstrap order (in `app::run`): logging → `load_dotenv` → `Storage::open` (+ migrations) → `SecretsStore` → `SettingsManager::load` → `EventBus` → `StateHub` → helper `HelperClient` → managers → `app.manage(AppCore)` → windows/panel → shortcuts → tray → `settings::side_effects::apply(None, &settings)`.
- Tests: the parity test passes; add `tests/integration/command-surface.test.ts`-style check for `EVENT_NAMES` (already there); Rust unit tests for pure pieces you add; a `cargo check --target aarch64-apple-darwin` (via `scripts/check-rust.sh --darwin`) in CI.
- Acceptance: on macOS `bun run tauri:dev` boots, HUD shows, mock provider answers with developer mode on, Settings persist across restarts.

### PR 1 — `bluey-protocols::gemini` (pure codec, fully unit-tested, no I/O)
Add `src-tauri/crates/bluey-protocols/src/gemini.rs` (+ `pub mod gemini;` in `lib.rs` and the module list in its doc comment):
- Constants: `API_BASE`, `LIVE_WS_URL_PREFIX`; URL builders `generate_url(base, model, stream)`, `embed_url`, `batch_embed_url`, `models_url(base, page_token)`, `live_url(api_key)` (document that the key is in the query string and must be redacted in logs).
- `GenerateBodyOptions { model, messages: &[AiMessage], max_output_tokens, temperature, output_schema, thinking_level: Option<ThinkingLevel> }` → `build_generate_body(&opts) -> Value` implementing §1.3 (system folding, role mapping, `inlineData` images, `$schema` stripping, no sampling params on `gemini-3*`).
- `thinking_level_for(task, latency, reasoning, model) -> Option<ThinkingLevel>` per §2.3; `is_gemini_3(model)`, `supports_minimal(model)`.
- Stream/unary parsing: `parse_response(data) -> GeminiResponse { text: String, finish: Option<String>, usage: Option<Usage{prompt, candidates, thoughts}>, block_reason: Option<String>, function_calls: Vec<FunctionCall> }`; `map_finish_reason(&str) -> FinishReason` (+ `blocked_code(&str)`); `parse_error_body(&str) -> Option<GeminiError { http_code, status, reason, retry_after: Option<Duration>, quota_id: Option<String> }>` and `map_gemini_error(http_status, Option<GeminiError>) -> BlueyError` per §1.6.
- Embeddings: `EmbedPurpose { Document { title: Option<String> }, Query }`, `embedding_text(purpose, text, model) -> String` (prefixes only for `gemini-embedding-2`), `build_batch_embed_body(model, texts, dims)`, `parse_batch_embeddings(json) -> Vec<Vec<f32>>`, `parse_single_embedding`.
- Models list: `parse_models_page(json) -> (Vec<ModelInfo { id (without "models/"), display_name, supported_methods, input_token_limit }>, Option<next_page_token>)`, plus `role_filter(role, &ModelInfo) -> bool` (`embedContent` for embedding, `bidiGenerateContent` + name contains `transcribe` for transcription, `generateContent` and not `*-tts*`/`*image*`/`*live*`/`*embedding*`/`*transcribe*` for text roles).
- Live: `LiveSetupOptions { model, language: Option<&str>, custom_vocabulary: &[String], smart_mode: bool, silence_ms }` → `live_setup_message`, `live_audio_message(base64_pcm, sample_rate)`, `live_audio_stream_end()`, `parse_live_message(&str) -> LiveEvent { SetupComplete, Interim(String), Final { text, language: Option<String> }, GoAway { time_left_ms: Option<u64> }, ResumptionUpdate { handle: Option<String>, resumable: bool }, Error(String), Other }`.
- Tests (mirror the style of `openai.rs`/`voice_live.rs` tests): body building for text-only, with images, with schema, with system messages; sampling params stripped for 3.x and kept for 2.5; thinking mapping table; chunk parsing (text, thought-skipping, finish, usage, blockReason, functionCall); error parsing incl. retryDelay and quotaId; embed body/response; models filtering; live frames round-trip. Use `pretty_assertions`.

### PR 2 — Gemini provider adapter, kind, presets, env import, docs
- `src-tauri/src/ai/providers/gemini.rs`: `GeminiProvider { http, base_url, api_key }` implementing `AiProvider`:
  - `stream()`: POST `streamGenerateContent?alt=sse` with header `x-goog-api-key`, `accept: text/event-stream`; on `status >= 400` read the body privately, `parse_error_body`, map, return; otherwise `spawn_gemini_sse(response, token)` in the pattern of `spawn_openai_sse` (deltas, usage incl. thoughts, finish, blocked → `Err`). Emit `StreamItem::Finished(Stop)` if the stream ends without a finish reason and no cancellation.
  - `embed(model, texts, purpose)`: batch in slices of ≤100 requests, `outputDimensionality` from settings (`ai.embeddingDimensions`, default 768), retry per §1.6, return vectors in order.
  - `list_models()`: paginate `GET /models?pageSize=200`, cache for the process lifetime (`OnceCell`/`Mutex<Option<…>>`) with a `refresh` path, return ids; expose `list_models_for_role(role)` on `AiManager` for the UI (new command `ai_list_models` gains an optional `role` param — update `commands.ts`, mock, tests).
- Extend `AiProvider::embed` signature with `purpose: EmbedPurpose` (Azure/OpenAI ignore it; Anthropic keeps returning `not_supported`); update `AiManager::embed(texts, purpose)` and the documents indexing (Document) vs retrieval (Query) call sites.
- `AiProviderKind::GoogleGemini` in `bluey-core/src/types/ai.rs`; `build_provider` arm; `router::provider_supports_vision` → true; router tests updated (`all_provider_kinds_support_vision`).
- `bluey_core::presets` (+ tests) and `AiSettings` additions: `bootstrap_provider: Option<String>`, `embedding_dimensions: u32` (default 768), `research_backend: ResearchBackend { Gemini, Claude }` (serde snake_case), all mirrored in `src/lib/types/settings.ts`, `src/lib/tauri/mock/fixtures.ts::createDefaultSettings()` and the Rust `Settings::default()` test.
- New command `ai_apply_provider_presets { providerId, overwrite }` (Rust + `commands.ts` + mock + parity test).
- `app::env_import` per §2.2 (unit-test the pure parts: env → provider configs/presets, precedence, alias `GOOGLE_API_KEY`). Import Exa/Firecrawl/Anthropic-agent keys into their fixed Keychain slots the same way.
- Error mapping: Gemini-aware `map_http_status` variant; extend `BlueyError` details with optional `retryAfterMs` if the type allows (check `bluey-core/src/error.rs`), so the UI can show "retry in 23 s".
- Docs: `.env.example` (§2.2), `README.md` stack line ("Google Gemini (default) · Microsoft Foundry / Azure OpenAI, Anthropic and OpenAI-compatible providers…"), `docs/AI_ARCHITECTURE.md` (new `google_gemini` adapter section with the §1.3/§2.3 rules, presets, switch), `docs/SECURITY.md` secret table row ("Google AI Studio key — Keychain `provider:gemini:api_key` — Rust adapters + sidecar env"), new `docs/adr/0007-gemini-default-provider.md` (decision, alternatives considered: Interactions API rejected for the Rust path because it stores requests server-side by default and Bluey manages history client-side; OpenAI-compat endpoint rejected because it has no transcription/Live; Rust crates `gemini-rust`/`genai` rejected because neither covers the Live API and the REST subset is small).
- Acceptance (macOS, real key): Settings → AI shows the auto-created "Google Gemini" provider with "Key saved", Test connection returns "Connected · gemini-3.8-flash · N ms", ⌘↵ on a screenshot streams an answer, classification/summarization run on `gemini-3.5-flash-lite`, a 429 shows a Retry banner with the delay.

### PR 3 — Embedding model tracking + reindex
- Migration `0003_embedding_model.sql`: add `embedding_model TEXT` and `embedding_dims INTEGER` to `document_chunks` (or `documents`), backfill NULL. `DocumentRepository::set_embedding(db, chunk_id, vec, model, dims)`; `chunks_with_embeddings(..., model_filter)` only returns chunks whose model matches the currently assigned embedding model; `retrieve()` passes the current model; `documents_reindex` re-embeds everything when the assignment (model or dims) changes — trigger it from `settings::side_effects` when `ai.models.embedding` or `ai.embeddingDimensions` changes and embeddings are enabled; show progress via `documents.*` events if they exist, else a `dev.log`.
- Tests in `bluey-storage` for the filter and the round-trip; cosine works unchanged on 768-dim vectors.

### PR 4 — Audio manager + `TranscriptionProvider` trait + Gemini Live STT
- `src-tauri/src/audio/mod.rs`: `AudioManager` (session lifecycle `start/stop/pause/resume/status/list_devices/test_microphone`, helper calls `audio.start{ microphone, systemAudio, sampleRate: 16000, chunkMs: 100 (cloud) | 200 (apple), emitPcm: <cloud>, vad, transcription:{ provider: apple|none }, levels }`, ring buffer + `TranscriptRepository` writes when `privacy.storeTranscripts`, `transcript.partial/final/cleared` + `audio.*` events, auto-start a `Session` on `audio_start` when none is active and end it on stop), plus `audio/transcription/{mod,apple,gemini_live,cloud_realtime,mock}.rs` behind `trait TranscriptionProvider { async fn start(&self, cfg) -> Result<()>; fn push_chunk(&self, chunk: HelperAudioChunk); async fn stop(&self); }`.
- `gemini_live.rs`: one `LiveSession` per audio source (mic, system) so speaker labels stay channel-derived ("You" vs the mode's other-party label — reuse `bluey_protocols::helper` speaker logic). State machine per session: `Connecting → SetupSent → Ready → Draining → Closed`; connect with `tokio_tungstenite::connect_async` (already a dependency; `native-tls`) to `live_url(key)`; send `setup`; buffer chunks until `setupComplete`; forward `realtimeInput.audio` frames; on helper VAD speech→silence lasting ≥ 500 ms send `audioStreamEnd: true` (hybrid VAD); map `Interim` → `transcript.partial` (replace current utterance for that source), `Final` → `TranscriptSegment { finalized: true, start_time/end_time from the chunk timestamps of the utterance, language }` → repository + `transcript.final`. **Rotation**: at 9 min 30 s of session age or on `goAway` open a replacement session in parallel, route new chunks to it once it is `Ready`, send `audioStreamEnd` to the old one, keep it open ≤ 2 s for trailing finals, then close — no audio gap, at most one duplicated final (dedupe by exact text + overlapping time). Reconnect on error with backoff 1 s → 30 s (max 5 attempts) then `audio.error{ code: "stt_unavailable", kind: network }` and fall back to Apple if the user allowed it. Redact the URL query in all logs. Never store raw audio (respect `privacy.storeRawAudio`).
- `cloud_realtime.rs`: same trait over the existing `voice_live`/`realtime` codecs (Azure alternate). `apple.rs`: passthrough of helper `transcript.*` events. `mock.rs`: fixture-driven.
- Settings/UI plumbing: `audio.transcriptionProvider` drives the helper config; `TranscriptionProviderKind::GeminiLive`. Optional (P2): `Mode.vocabulary?: string[]` → `customVocabulary` (≤100 terms) from the active mode.
- Optional PR 4b: `ai_transcribe_file { path, diarization, wordTimestamps }` for imported recordings via `gemini-3.5-transcribe` (`inlineData` ≤ 20 MB, else Files API resumable upload from §D of the reference), producing `TranscriptSegment`s with `speaker = spk_n` and a `SessionEvent`. Only if time permits; note it in the docs as optional.
- Tests: codec tests already in PR 1; add Rust tests for the rotation/dedupe logic with an injected clock and a fake socket (trait-abstract the socket); TS tests unaffected.
- Acceptance (macOS, real key): ⌘⇧L with `gemini_live` shows partials within ~1 s of speech, finals on pauses, survives a 12-minute run without gaps, and falls back to Apple with a visible error when the key is removed.

### PR 5 — Research sidecar: Gemini backend
- `sidecars/agent/src/config.ts`: `backend: "gemini" | "claude"` from `RESEARCH_BACKEND` (default `gemini`), `geminiApiKey` from `GEMINI_API_KEY` ?? `GOOGLE_API_KEY`, `geminiModel` from `BLUEY_RESEARCH_MODEL` ?? `gemini-3.8-flash`; `checkModelCredentials` names `GEMINI_API_KEY` for the gemini backend; add `GEMINI_API_KEY`/`GOOGLE_API_KEY` to `PROVIDER_ENV_VARS` so they are stripped from the Claude subprocess env.
- `sidecars/agent/src/agent.ts`: extract the Claude `query()` block into `runClaude()`; add `runGemini()` in new `src/gemini.ts` using the **same** `handlers`, `CitationStore`, `DocumentBroker`, `ProtocolWriter`, `buildSystemPrompt`, `maxTurns`, `AbortController`. Loop: `contents = [user prompt]`; each turn `generateContentStream({ model, contents, config: { systemInstruction, tools: [{ functionDeclarations }], thinkingConfig: { thinkingLevel: "low" }, abortSignal, httpOptions: { timeout: 60_000, retryOptions: { attempts: 3 } } } })`; stream text → `research.textDelta`; collect `functionCall` parts (keep `id`); push the model `content` unchanged; execute handlers; push one `user` turn with all `functionResponse` parts (`id` echoed) → `research.toolCall` + `research.progress`; stop when a turn has no function calls or `turns == maxTurns` (`max_turns_exceeded`); final turn without tools: `responseMimeType: "application/json"`, `responseJsonSchema: REPORT_OUTPUT_SCHEMA`, `thinkingLevel: "medium"` → parse → `CitationStore.finalize` → `research.completed` with `usage` from `usageMetadata` (`promptTokenCount`, `candidatesTokenCount + thoughtsTokenCount`). Errors: `ApiError.status` 400 with `API_KEY_INVALID` → `missing_api_key`-style configuration failure naming `GEMINI_API_KEY`; 429 → `budget_exceeded`-like `rate_limited` (new code, kind `research`); abort → `cancelled`. Function declarations: convert the existing zod shapes with `z.toJSONSchema` (zod v4 is a dependency) into `parametersJsonSchema` (strip `$schema`).
- Dependencies: add `@google/genai` `^2.21.0 <3` to `sidecars/agent/package.json` **and** root `package.json` (vitest/tsc resolve sidecar sources from the root). Mock mode: add `createMockGeminiTurns()` so `BLUEY_AGENT_MOCK=1 RESEARCH_BACKEND=gemini bun src/main.ts` exercises the loop offline (keep the document round-trip).
- Build: add `entry-darwin-{arm64,x64}-lite.ts` without the `with { type: "file" }` Claude import, and a `BLUEY_AGENT_VARIANT=lite|full` switch in `scripts/build-agent.sh` (lite skips the Claude package existence check). Default build = lite (few MB) when `RESEARCH_BACKEND=gemini`; full when Claude is wanted. Update `sidecars/agent/README.md` and `docs/AGENT_SIDECAR_PROTOCOL.md` (protocol unchanged; add the `GEMINI_API_KEY` env row and `rate_limited` code).
- Rust `AgentManager` (from PR 0) passes the env described in §2.5 and forwards `research.started.model` / `completed.usage` if you extend `bluey_protocols::agent` (optional).
- Tests: `tests/sidecar/config.test.ts` (backend/credential cases), `tests/sidecar/sidecar.test.ts` (missing-key case names `GEMINI_API_KEY` by default; keep a `RESEARCH_BACKEND=claude` variant), new `tests/sidecar/gemini.test.ts` with an injected `generateFn` scripting functionCall → functionResponse → JSON report, cancellation mid-loop, max-turns.
- Acceptance: a "deep dive" query from the HUD streams progress and returns a report with only tool-observed citations using the Gemini key alone.

### PR 6 — Frontend: provider UX for the Gemini default + switch
- Types/fixtures: `"google_gemini"` in `AIProviderKind`; `"gemini_live"` in `TranscriptionProviderKind`; `AISettings.{bootstrapProvider?, embeddingDimensions, researchBackend}`; `FIXTURE_MODELS_BY_KIND.google_gemini` and a default Gemini provider + Gemini presets in `createDefaultSettings()` (`src/lib/tauri/mock/fixtures.ts`) so `bun run dev` exercises the new UI; mock implementations for `ai_apply_provider_presets` and `ai_list_models{role}`.
- `ProviderCard.tsx` / `provider-form.ts` / `AITab.tsx`: kind picker lists "Google Gemini (AI Studio)" **first** and is the default for new providers; per-kind fields (Gemini: optional Base URL with placeholder `https://generativelanguage.googleapis.com/v1beta`; Azure: API version + deployments; Anthropic/OpenAI: Base URL); relax the "Base URL required" Save guard for Gemini; **delete provider** action (with confirm) that also calls `secrets_delete`; empty state when no providers; badges showing which roles use each provider; "Default provider" select + "Use recommended models" (→ `ai_apply_provider_presets`); `SecretKeyField` variant for Gemini labelled "Google AI Studio API key", placeholder `AIza…`, helper link to `https://aistudio.google.com/apikey` via `openExternal`; model role rows become controlled inputs with a role-filtered datalist from `ai_list_models{role}`, loading + "Couldn't load models" states; Test connection uses `useErrorPresenter` and offers a model pick when none is assigned (never swallow thrown errors); role hints become provider-neutral (drop "MAI-Transcribe-1.5 (Voice Live)", "Claude agent", "Anthropic (agent)").
- Research section: "Research agent backend" select `{ gemini (default), claude }` bound to `ai.researchBackend`; explain that Gemini research reuses the Gemini key while Claude needs `agent:anthropic:api_key` or Foundry; keep Exa/Firecrawl fields.
- `AudioTab.tsx`: transcription provider options `apple | gemini_live | cloud_realtime`; dynamic description showing the assigned transcription model + provider, a warning when that provider has no key, and the Gemini live notes (16 kHz, sessions rotate every ~10 min, no speaker diarization on live).
- Onboarding: new `connect-ai` step after `name`: paste the AI Studio key → create the `gemini` provider if missing → `secrets_set` → `ai_apply_provider_presets` → auto `ai_test_connection` with the default model; links "Use another provider" (→ Settings → AI) and "Skip". `TestAIStep` copy: "Add your Google AI Studio key (recommended) or another provider".
- Error presenter: messages for `config.api_key_invalid`, `config.model_not_found`, `network.http_429` (show retry delay; daily-quota wording), `ai.blocked_*`, `audio.stt_unavailable`/`stt_fallback`.
- Tests: `tests/ui/settings.test.tsx` (kind picker default, per-kind fields, delete, presets), `tests/ui/onboarding.test.tsx` (key step), `tests/ui/mock-transport.test.ts` (new commands), `tests/unit/*` for any pure helpers.

### PR 7 — Frontend: close the "fully built" gaps (P0 first, then P1)
P0:
1. **Proactive preparation loop**: subscribe to `transcript.final` (+ `transcriptStore.questions` / `question.detected`) in a new `src/stores/proactive.ts` (or `initStores.ts`), run `engine.classify()` then `engine.prepare()` when `ai.proactivePreparation` is on, publish `response.prepared` so the existing ⌘⇧↵ path has a producer; make `takePrepared()` pick the prepared response for the question currently shown.
2. **Live transcript strip in the HUD**: rolling last N segments + current partial from `transcriptStore`, speaker label + confidence, collapsible, in `src/features/hud/`.
3. **Error surfacing**: subscribe to `app.error`, `audio.error`, `helper.status` → toast/`ErrorBanner` via `useErrorPresenter`; recovery button on the `StatePill` error state using `AppStatus.error.recovery`; stop swallowing thrown errors in `ProviderCard`, `TestAIStep`, `SecretKeyField`, `AudioTab`.
4. **Session controls**: start/pause/resume/end from the HUD toolbar or menu (or rely on the PR 4 auto-session and show it), active session in pill/tooltips, rename/delete in `SessionsTab`.
5. **"My Context" documents UI**: resume / job description / notes upload with `DocumentKind`, list/delete/reindex, `documents_add` with global scope (reuse `ModeFilesDropzone` with a kind picker) so retrieval's `resume`/`job_description` kinds can exist.
P1: deep-research progress phase (`EnginePhase "researching"`, forward `research.event` progress/tool calls into the current turn with a cancel → `research_deep_cancel`); Appearance settings tab (theme, opacity, width, blur, font size, density, position, follow display, reduced motion); custom-mode editor completeness (description, icon, `responseSchema`, `contextRequirements`, group); shortcut enable/disable toggle; `rawAudioRetentionMinutes` input; enforce `privacy.cloudAiEnabled` in `engine.ts` before `ai_stream`; fix `outputLanguage` value mismatch (`"en"` vs `"English"`); persist HUD `screenEnabled`. P2: About links, session export to file, HUD history popover.
Tests: extend `tests/ui/hud.test.tsx`, `tests/ui/stores.test.ts`, `tests/ui/settings.test.tsx` accordingly.

### PR 8 — Docs, release, CI
- Update `docs/AUDIO_ARCHITECTURE.md` (Gemini Live path, rotation, hybrid VAD), `docs/AI_ARCHITECTURE.md` (research backends, presets, switch), `docs/DEVELOPMENT.md` (env quick start with one key; `BLUEY_AGENT_VARIANT`), `docs/TESTING.md` (new suites; how to run the Gemini smoke tests), `README.md` quick start (`GEMINI_API_KEY` is the only key needed).
- CI (GitHub Actions; if the integration token cannot push workflow files, prepare them and ask the user to commit): `bun install --frozen-lockfile`, `bun run typecheck`, `bun run lint`, `bun run test`, `bun run build`, `bun run check:rust --darwin`; macOS job: `bun run build:helpers` + `cargo check` of the app crate.
- `scripts/release.sh`: build the lite agent by default.

---

## 4. Detailed specifications

### 4.1 `bluey-protocols::gemini::build_generate_body` — behaviour spec
1. Partition `messages`: all `AiRole::System` → join `text_content()` with `"\n\n"` → `systemInstruction.parts[0].text` (omit when empty). Others → `contents[]` with `role: "user" | "model"`; text parts → `{ "text" }`; image parts → `{ "inlineData": { "mimeType": media_type.as_str(), "data" } }`. Preserve part order. If two consecutive contents share a role, keep them (Gemini accepts it), but add a test documenting the behaviour.
2. `generationConfig`: `maxOutputTokens` if set; `thinkingConfig.thinkingLevel` if `Some` and not `medium` (default; omitting keeps the body small); when `output_schema` is `Some`: `responseMimeType: "application/json"` and `responseJsonSchema` = schema with `$schema` removed (recursively leave everything else intact); `temperature` only if `Some` **and** `!is_gemini_3(model)`.
3. Never include `topP`, `topK`, `candidateCount`, `thinkingBudget`, `responseSchema`.

### 4.2 `spawn_gemini_sse` — behaviour spec
Consume `response.bytes_stream().eventsource()`; for each frame `parse_response(&frame.data)`: emit `Delta` for non-empty text; on `usage` emit `Usage { input: prompt, output: candidates + thoughts }` (last one wins upstream); on `block_reason` or an error-class finish reason send `Err(BlueyError::ai("blocked_<reason>", "the model refused to answer (<reason>)"))` and return; on `STOP`/`MAX_TOKENS` emit `Finished`. Tolerate unknown frames. On transport error mid-stream: `Err(network "stream")`. Honour `token.cancelled()` in the `select!` like the other adapters.

### 4.3 Embedding call spec
`AiManager::embed(texts, purpose)` → adapter batches of ≤100; each request `content.parts[0].text = embedding_text(purpose, text, model)`; `outputDimensionality = settings.ai.embedding_dimensions`; validate every returned vector has that length (else `ai.embeddings_parse`); the `documents` manager records `(model, dims)` with each vector (PR 3).

### 4.4 Gemini Live session rotation — behaviour spec
- `SESSION_SOFT_LIMIT = 9m30s`, `DRAIN_TIMEOUT = 2s`, `SILENCE_FOR_STREAM_END = 500ms`.
- Each source owns `current: LiveSession` and `next: Option<LiveSession>`. At `age >= SOFT_LIMIT` (or `GoAway`): spawn `next`; while `next` is not `Ready`, chunks continue to `current`; when `next` is `Ready`, swap, send `audioStreamEnd` to the old session, keep reading it for `DRAIN_TIMEOUT`, then close.
- Dedupe: if the first final from `next` equals (trimmed) the last final of the old session and their time windows overlap, drop it.
- Timestamps: `start_time` = `startMs` of the first chunk after the previous final (or after the interim reset), `end_time` = `endMs` of the last chunk before the final arrives; times are ms since the audio session started (helper semantics).
- Language: `audio.transcriptionLanguage == "auto"` → `languageCodes: []` else `[tag]`.
- Metrics: emit `dev.metrics.transcript_ms` = time from `endMs` of the utterance's last chunk to the final event.

### 4.5 Command / event surface additions (mirror in `commands.ts`, `events.ts`, mock transport, Rust)
`ai_apply_provider_presets { providerId, overwrite } -> Settings`; `ai_list_models { providerId, role? } -> string[]`; optional `ai_transcribe_file { path, diarization, wordTimestamps, sessionId? } -> TranscriptSegment[]`. No new event names are required; if you add any, update both `EVENT_NAMES` and `BlueyEvent::name()` (the parity test checks it).

---

## 5. Verification

### 5.1 Local commands (every PR)
```
bun install
bun run typecheck && bun run lint && bun run test && bun run build
bun run check:rust --darwin        # fmt + tests + clippy (-D warnings) + app-crate cargo check
bun run test:rust
bun run build:agent host            # sidecar compiles (lite variant by default)
bun run tauri:dev                   # macOS only
```

### 5.2 Live-API smoke tests (real key; run once per PR 2/3/4/5 and record results in the PR description — beware the free tier's low RPM/RPD)
```bash
export GEMINI_API_KEY=...
curl -s -H "x-goog-api-key: $GEMINI_API_KEY" "https://generativelanguage.googleapis.com/v1beta/models?pageSize=5" | head -c 600
curl -s -H "x-goog-api-key: $GEMINI_API_KEY" -H "content-type: application/json" \
  -d '{"contents":[{"role":"user","parts":[{"text":"Reply with the single word: ok"}]}],"generationConfig":{"maxOutputTokens":8,"thinkingConfig":{"thinkingLevel":"low"}}}' \
  "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:generateContent"
curl -sN -H "x-goog-api-key: $GEMINI_API_KEY" -H "content-type: application/json" \
  -d '{"contents":[{"role":"user","parts":[{"text":"Count to five, one number per line."}]}]}' \
  "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:streamGenerateContent?alt=sse" | head -20
curl -s -H "x-goog-api-key: $GEMINI_API_KEY" -H "content-type: application/json" \
  -d '{"requests":[{"model":"models/gemini-embedding-2","content":{"parts":[{"text":"title: none | text: hello"}]},"outputDimensionality":768}]}' \
  "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-2:batchEmbedContents" | head -c 300
```
Live transcription: use a tiny Bun script with `@google/genai` (`ai.live.connect({ model: "gemini-3.5-transcribe-live", config: { responseModalities: [Modality.TEXT], inputAudioTranscription: { languageCodes: [] } }, callbacks })`) feeding a 16 kHz PCM WAV in 100 ms chunks; confirm interim + final events, then confirm the same with the Rust `LiveSession` against the same file (add it as an ignored `cargo test -- --ignored` requiring the env var).

### 5.3 Manual QA script (macOS, real key)
1. Fresh profile: `cp .env.example .env`, set only `VITE_CLERK_PUBLISHABLE_KEY` and `GEMINI_API_KEY`, `bun run tauri:dev` → onboarding key step pre-filled state "Key saved", test passes.
2. Settings → AI: Gemini provider present, presets applied, Default provider = Google Gemini, model lists load per role, Test connection OK; remove the key → correct configuration error + recovery.
3. HUD: ⌘↵ screenshot question (vision), plain question (fast), "design a rate limiter" in System Design mode (reasoning → thinkingLevel high), a deep-dive research query (progress + citations), ⌘⇧L live transcript with partials/finals for ≥ 12 minutes (rotation), question detection → prepared response → ⌘⇧↵.
4. Documents: upload a resume (My Context), confirm embeddings indexed with `gemini-embedding-2`/768, retrieval used in an Interview-mode answer; change dims → reindex runs.
5. Switch: set `BLUEY_AI_PROVIDER=azure_foundry` + Azure key, restart → roles re-pointed to Azure presets, transcription `cloud_realtime`; switch back at runtime via Settings → Default provider. Verify nothing logs a key (`~/Library/Logs/Bluey/`).
6. Offline: disconnect network → "Bluey is offline", Apple STT fallback message, no crash.

### 5.4 Decision rule for `@google/genai` under Bun
The SDK's Bun compatibility is undocumented. In PR 5, first run `bun test tests/sidecar` and the compiled lite binary in mock mode, then a real `generateContentStream` and `live.connect` from the compiled binary. If either fails under Bun (e.g. `ws`/`google-auth-library` issues), replace the SDK in the sidecar with a ~200-line REST client using `fetch` + an SSE line parser against the shapes in §1.3 (keep the same `runGemini()` interface and tests). Record the outcome in `sidecars/agent/README.md`.

---

## 6. Definition of done
- [ ] `tests/integration/command-surface.test.ts` passes; `bun run check:rust --darwin` passes; all TS suites green; `bun run build` produces `dist/`.
- [ ] With only `GEMINI_API_KEY` set: chat, vision, live transcription, embeddings/retrieval and deep research all work; no other provider needed.
- [ ] `BLUEY_AI_PROVIDER` and Settings → Default provider switch every role (and transcription/research backends) between Gemini and Azure/Foundry/Anthropic/OpenAI-compatible without code changes.
- [ ] No `temperature/topP/topK` ever reaches a `gemini-3*` model; `thinkingLevel` mapping is unit-tested; `minimal` is never sent to 3.7/3.8.
- [ ] Live STT sessions rotate before the 10-minute cap with no audio gap; errors fall back to Apple with a visible, recoverable message.
- [ ] Keys exist only in the Keychain and in sidecar process envs; nothing secret in SQLite, logs, or the WebView; the Live URL query string is redacted in logs.
- [ ] Docs (`.env.example`, README, AI/AUDIO/SECURITY architecture docs, ADR 0007, sidecar README) match the code.
- [ ] Every PR has a Greptile 5/5 review before merge.

## 7. Explicitly out of scope / do not do
- Do not use the Interactions API, Managed Agents, Google Search grounding, image/video/TTS generation, or Vertex AI.
- Do not use the OpenAI-compatibility endpoint as the primary Gemini path (no transcription/Live; beta) — at most as a debugging spike.
- Do not add `@google/genai` to the frontend bundle or call Gemini from the WebView; do not mint ephemeral tokens unless a later phase moves Live sessions into the WebView.
- Do not rewrite the existing Azure/Anthropic/OpenAI adapters, the TS intelligence layer, or the Swift helper beyond the `chunkMs`/`emitPcm` parameters they already support.
- Do not store raw audio, prompts, or provider bodies; do not weaken the secret boundary (ADR 0001).
