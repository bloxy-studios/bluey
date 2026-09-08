# AI Architecture

## Layers

```
AskInput (UI)  ──►  Response Engine (TS)  ──►  AIRequest  ──►  ai_stream (Rust)  ──►  Provider
                       │        ▲                                 │  Model Router
                       │        └── AIChunk stream (Channel) ◄────┘
                       ▼
             Response Optimizer ──► BlueyResponse ──► HUD / storage
```

### Intelligence layer (TypeScript, `src/ai`, `src/context`, `src/modes`, `src/transcript`)

- **Context fusion** (`src/context/fusion.ts`): every source becomes a scored `ContextItem`
  (`user_instruction`, `transcript`, `transcript_old`, `ocr`, `accessibility`, `resume`,
  `job_description`, `document`, `session_memory`, `personal_instructions`).
- **Token budget** (`src/context/budget.ts`): priorities per spec — current question ≫ recent
  transcript / OCR / focused UI > JD / resume > old transcript. Over budget → compress (tail for
  transcript, head for OCR), summarize, drop lowest score; never blindly truncate from the end.
- **Intent classification** (`src/context/relevance.ts`): chooses the `AITask`, whether vision
  is required, reasoning depth and latency budget, and the response schema.
- **Prompt architecture** (`src/ai/prompt-builder.ts`, `src/ai/prompts/*`): separate parts —
  system · mode · style · context · task · output schema. No giant strings elsewhere.
- **Structured output** (`src/modes/schemas.ts`): zod schemas → JSON Schema per response type;
  providers that support JSON schema use it natively, others are prompted and parsed tolerantly.
- **Streaming** (`src/ai/stream.ts`): drafts render progressively; unterminated code fences are
  buffered so malformed code is never shown as final.
- **Generations** (`src/ai/generations.ts`): stale-response protection (ADR 0005).
- **Optimizer** (`src/ai/optimizer.ts`): remove repetition/filler, respect length & tone,
  preserve code, caveats and citations. Goal: the minimum text necessary to be useful.
- **Proactive preparation**: when the classifier detects a likely question with
  `requiresResponse`, the engine silently prepares and caches a response; ⌘⇧↵ shows it
  instantly. Results are never auto-displayed.

### Orchestration layer (Rust, `src-tauri/src/ai`)

- **Model router** (`bluey_core::router`): `select(task, latency, reasoning, contextTokens,
visionRequired, preferredRole)` over the user's role assignments
  (`default/fast/reasoning/vision/research/transcription/embedding`) with fallbacks. Default
  strategy: classification → fast · answer → fast/balanced · coding → default · system design →
  reasoning · research → agent · summarization → fast. All model identifiers are configuration.
- **Provider presets** (`bluey_core::presets`): one table of reserved provider ids
  (`gemini`, `azure-foundry`, `anthropic`, `openai`), display names and the recommended model
  per role. `ai_apply_provider_presets { providerId, overwrite }` points roles at a provider's
  presets (`overwrite = false` fills only unassigned roles); the `.env` import
  (`app::env_import`, planned by `presets::plan_env_import`) applies the nominated provider's
  presets at boot and honours `BLUEY_MODEL_*` overrides. `ai_list_models { providerId, role? }`
  narrows a provider's catalogue to models fit for a role.
- **Provider adapters** (`AIProvider` trait: `stream(request) -> Stream<AIChunk>`,
  `embed(model, texts, purpose)`, `list_models(role?)`; connection tests are a tiny `stream`):
  - `google_gemini` — **the default** (ADR 0007). Gemini API over REST with the AI Studio key
    in `x-goog-api-key` (never `?key=`): `POST /v1beta/models/{model}:streamGenerateContent?alt=sse`
    with bodies from `bluey_protocols::gemini` — roles `user`/`model`, `systemInstruction`,
    inline base64 images, `responseJsonSchema` (top-level `$schema` stripped), and on 3.x models
    **only** `thinkingConfig.thinkingLevel` (never `temperature`/`topP`/`topK`/`candidateCount`/
    `thinkingBudget`). Thinking policy: `deep` reasoning, `deep_reasoning`, `system_design` with
    reasoning ≥ light or a `deep` latency budget → `high`; `classification` / `ultra-fast` →
    `minimal` on `gemini-3.5-flash-lite` (else `low`); `answer`/`vision` at `fast` → `low`;
    everything else `medium` (the default, omitted). Embeddings via `batchEmbedContents` on
    `gemini-embedding-2`, MRL-truncated to `ai.embeddingDimensions` (768 default), with the
    documented prompt prefixes (`title: {title|none} | text: …` for chunks, `task: search result |
    query: …` for queries). Model listing pages `GET /models` and filters by role
    (`embedContent`, `bidiGenerateContent`+`transcribe`, or `generateContent` minus TTS/image/
    live/embedding/transcribe ids). Presets: `gemini-3.8-flash` (default/reasoning/vision/
    research), `gemini-3.5-flash-lite` (fast), `gemini-3.5-transcribe` (batch transcription),
    `gemini-embedding-2`.
    Error mapping: 400 `API_KEY_INVALID` → `config.api_key_invalid`; other 400 →
    `ai.invalid_request`; 403 → `config.http_403`; 404 → `config.model_not_found`; 429 →
    `network.http_429` with `details.retryAfterMs` and `details.dailyQuota` (a `quotaId`
    containing `PerDay`); 5xx → `network.http_5xx`; refusals (`promptFeedback.blockReason` or an
    error-class `finishReason`) → `ai.blocked_<reason>`. Retries: up to 3 attempts on 429/5xx
    honouring `retryDelay` (never on 400/403/404); streams retry only before the first byte.
  - `azure_foundry` — Microsoft Foundry's OpenAI-compatible **v1 GA** endpoint
    `https://{resource}.openai.azure.com/openai/v1/chat/completions` (no `api-version`;
    `api_version = "preview"` opts into v1 preview features, a dated value selects the legacy
    `/openai/deployments/{d}/…` form), `api-key` header, `model = deployment`,
    `stream_options.include_usage`, `response_format: json_schema`, vision via `image_url`
    data URLs, embeddings via `/openai/v1/embeddings`. Current ids: `gpt-6-astra`,
    `gpt-5.6-sol` / `-terra` / `-luna`, `text-embedding-3-small|large`.
  - `anthropic` — Messages API (`anthropic-version: 2023-06-01`), SSE events
    `message_start/content_block_delta/message_delta/message_stop`, base64 image blocks,
    structured output via `output_config.format = json_schema`. Also serves **Claude in
    Microsoft Foundry**: base URL `https://{resource}.services.ai.azure.com/anthropic`, same
    resource key (Foundry accepts `x-api-key`), `model` = deployment name
    (`claude-opus-5`, `claude-sonnet-5`, `claude-haiku-4-5`).
  - `openai_compatible` — any `/v1/chat/completions` (Bearer auth).
  - `mock` — deterministic streamed answers for development and tests (clearly isolated; only
    selectable when developer mode is on).
- **Cancellation**: each request has a `CancellationToken`; `ai_cancel(requestId)` aborts the
  HTTP stream; newer generations cancel older ones.
- **Metrics**: time to first token, total latency, token usage → `ai_requests` table +
  `dev.metrics` event.

### Transcription providers (`TranscriptionProvider` trait)

`src-tauri/src/transcription/`: `TranscriptionProvider::open(options, sink) → TranscriptionSession`
(`push_audio(PcmChunk)`, `close()`), one session per audio source. The audio manager forwards the
helper's PCM16 16 kHz chunks and maps `Interim` / `Final` / `Failed` events back onto transcript
segments; a provider that cannot run falls back to Apple with `audio.error{stt_fallback}`.

- `gemini_live` (**default**) — Gemini Live API, `gemini-3.5-transcribe-live`, with the Google AI
  Studio key from `provider:gemini:api_key`. Session rotation at 9 min 30 s / `goAway` with a
  2 s drain and final dedupe; `audioStreamEnd` after 500 ms of silence; configuration errors
  map to `config.api_key_invalid` / `config.model_not_found`. See `AUDIO_ARCHITECTURE.md`.
- `apple` — on-device `SFSpeechRecognizer` inside the helper; partial/final events.
- `cloud_realtime` — live cloud STT over WebSocket. The transport is chosen from
  the transcription-role model id (`bluey_protocols::voice_live::transport_for_model`):
  - **MAI-Transcribe-1.5** (`MAI-Transcribe-1.5`, `mai-transcribe`, or the Foundry
    catalog URI `azureml://registries/azureml-cogsvc/models/MAI-Transcribe-1.5/versions/2026-06-02`) →
    Foundry **Voice Live**. The catalog URI and version are **not** sent on the wire.
    Voice Live gets `input_audio_transcription.model = mai-transcribe` (service default = 1.5).
    Fast Transcription REST (Foundry playground / file upload) uses
    `enhancedMode.model = "MAI-Transcribe-1.5"` on
    `POST …/speechtotext/transcriptions:transcribe?api-version=2025-10-15` — that is
    WAV/MP3/FLAC, not live PCM. **MAI-Transcribe-2** aliases still map if the project
    has that model; Bluey's default is 1.5 because 2 is not on every Foundry project.
    The WebSocket `model=` query is a Voice Live _companion_ (`BLUEY_MODEL_VOICE_LIVE`,
    default `gpt-4.1-mini`) — fully managed, not a Foundry deployment, and **not**
    `gpt-5.6-luna` / `gpt-6-astra` (those are not on the Voice Live companion list).
    PCM16 @ 16 kHz matches helper capture. Auth: Foundry `api-key` header.
  - **OpenAI STT** (`gpt-4o-mini-transcribe`, `gpt-4o-transcribe`,
    `gpt-4o-transcribe-diarize`, `gpt-realtime-whisper`) → Azure/OpenAI
    `/openai/v1/realtime?intent=transcription`. Often **not deployed** on Foundry
    resources; Bluey's default is MAI-Transcribe-1.5.
    File-based Azure Speech Fast Transcription is the Foundry playground path (WAV/MP3/FLAC),
    not live meetings. Codec: `bluey_protocols::voice_live` + shared `realtime::parse_event` /
    `append_audio`; manager: `src-tauri/src/transcription/cloud_realtime.rs` (Voice Live over
    `api-key`; the OpenAI realtime transport is rejected as `not_supported` because it expects
    24 kHz audio while the helper captures 16 kHz).
- `mock` — fixture-driven for tests.
  Speaker labels derive from the audio channel (`microphone` → "You", `system` → other party,
  labelled per mode) with explicit confidence; Bluey never claims certain diarization.

**Batch (`ai_transcribe_file`)** — a whole recording (WAV, MP3, AIFF, AAC, OGG, FLAC) goes through
the transcription-role provider in one `generateContent` call on `gemini-3.5-transcribe` with
`audioTranscriptionConfig { languageCodes, diarization, wordTimestamp }` (`AiProvider::transcribe_audio`;
other providers answer `not_supported`). Files up to 14 MB travel inline as `inlineData`; larger ones
use the Files API resumable upload (`upload/v1beta/files`, polled until `ACTIVE`) and are deleted right
after the call. `transcription::batch` turns the `audioTranscription` speaker turns into finalized
`TranscriptSegment`s (`speaker = spk_n`, cut on speaker change / 1.5 s pauses / 40 words, estimated
timings when the model returns none), stores them when privacy → store transcripts is on, and adds a
`recording_imported` event to the session — the one given, or a new completed
"Imported · <file>" session. Settings → Sessions → *Import recording…* / *Add recording*.

### Research

Research Router → `none | search | search_scrape | deep_agent` (ADR 0004). `search`/`scrape`
run in Rust (Exa `POST /search`, Firecrawl `POST /v2/scrape`); `deep_agent` spawns the Bun
sidecar with the Claude Agent SDK and scoped tools. Public queries only. The Agent SDK is
Claude-only (it drives Claude Code) — on Foundry via `CLAUDE_CODE_USE_FOUNDRY=1` and the
`ANTHROPIC_FOUNDRY_*` variables (see `sidecars/agent/README.md`); it is never used for the
latency-sensitive live paths, which stay on the direct provider adapters above.

### Offline behaviour

If no provider is reachable the HUD shows "Bluey is offline"; mode switching, settings, session
history, transcript display, screen capture and local OCR keep working. Requests fail fast with
`BlueyError{kind: network}` and a Retry action.

### Performance targets (targets, not guarantees)

shortcut reaction < 100 ms · capture < 200 ms · OCR < 500 ms · context assembly < 300 ms ·
fast model first token < 1 s where the provider allows · normal answer 2–3 s · transcript partial
< 1 s. Dev overlay shows each stage.
