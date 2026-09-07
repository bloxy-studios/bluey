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
* **Context fusion** (`src/context/fusion.ts`): every source becomes a scored `ContextItem`
  (`user_instruction`, `transcript`, `transcript_old`, `ocr`, `accessibility`, `resume`,
  `job_description`, `document`, `session_memory`, `personal_instructions`).
* **Token budget** (`src/context/budget.ts`): priorities per spec — current question ≫ recent
  transcript / OCR / focused UI > JD / resume > old transcript. Over budget → compress (tail for
  transcript, head for OCR), summarize, drop lowest score; never blindly truncate from the end.
* **Intent classification** (`src/context/relevance.ts`): chooses the `AITask`, whether vision
  is required, reasoning depth and latency budget, and the response schema.
* **Prompt architecture** (`src/ai/prompt-builder.ts`, `src/ai/prompts/*`): separate parts —
  system · mode · style · context · task · output schema. No giant strings elsewhere.
* **Structured output** (`src/modes/schemas.ts`): zod schemas → JSON Schema per response type;
  providers that support JSON schema use it natively, others are prompted and parsed tolerantly.
* **Streaming** (`src/ai/stream.ts`): drafts render progressively; unterminated code fences are
  buffered so malformed code is never shown as final.
* **Generations** (`src/ai/generations.ts`): stale-response protection (ADR 0005).
* **Optimizer** (`src/ai/optimizer.ts`): remove repetition/filler, respect length & tone,
  preserve code, caveats and citations. Goal: the minimum text necessary to be useful.
* **Proactive preparation**: when the classifier detects a likely question with
  `requiresResponse`, the engine silently prepares and caches a response; ⌘⇧↵ shows it
  instantly. Results are never auto-displayed.

### Orchestration layer (Rust, `src-tauri/src/ai`)
* **Model router** (`bluey_core::router`): `select(task, latency, reasoning, contextTokens,
  visionRequired, preferredRole)` over the user's role assignments
  (`default/fast/reasoning/vision/research/transcription/embedding`) with fallbacks. Default
  strategy: classification → fast · answer → fast/balanced · coding → default · system design →
  reasoning · research → agent · summarization → fast. All model identifiers are configuration.
* **Provider adapters** (`AIProvider` trait: `stream(request) -> Stream<AIChunk>`, `embed`,
  `test_connection`, `list_models`):
  * `azure_foundry` — OpenAI-compatible v1 endpoint
    `https://{resource}.openai.azure.com/openai/v1/chat/completions`, `api-key` header,
    `model = deployment`, `stream_options.include_usage`, `response_format: json_schema`,
    vision via `image_url` data URLs, embeddings via `/openai/v1/embeddings`.
  * `anthropic` — Messages API (`anthropic-version: 2023-06-01`), SSE events
    `message_start/content_block_delta/message_delta/message_stop`, base64 image blocks,
    structured output via `output_config.format = json_schema`.
  * `openai_compatible` — any `/v1/chat/completions` (Bearer auth).
  * `mock` — deterministic streamed answers for development and tests (clearly isolated; only
    selectable when developer mode is on).
* **Cancellation**: each request has a `CancellationToken`; `ai_cancel(requestId)` aborts the
  HTTP stream; newer generations cancel older ones.
* **Metrics**: time to first token, total latency, token usage → `ai_requests` table +
  `dev.metrics` event.

### Transcription providers (`TranscriptionProvider` trait)
* `apple` (default) — on-device `SFSpeechRecognizer` inside the helper; partial/final events.
* `cloud_realtime` — OpenAI/Azure realtime transcription over WebSocket
  (`…/openai/v1/realtime?intent=transcription`), PCM16 chunks from the helper (`emitPcm`).
* `mock` — fixture-driven for tests.
Speaker labels derive from the audio channel (`microphone` → "You", `system` → other party,
labelled per mode) with explicit confidence; Bluey never claims certain diarization.

### Research
Research Router → `none | search | search_scrape | deep_agent` (ADR 0004). `search`/`scrape`
run in Rust (Exa `POST /search`, Firecrawl `POST /v2/scrape`); `deep_agent` spawns the Bun
sidecar with the Claude Agent SDK and scoped tools. Public queries only.

### Offline behaviour
If no provider is reachable the HUD shows "Bluey is offline"; mode switching, settings, session
history, transcript display, screen capture and local OCR keep working. Requests fail fast with
`BlueyError{kind: network}` and a Retry action.

### Performance targets (targets, not guarantees)
shortcut reaction < 100 ms · capture < 200 ms · OCR < 500 ms · context assembly < 300 ms ·
fast model first token < 1 s where the provider allows · normal answer 2–3 s · transcript partial
< 1 s. Dev overlay shows each stage.
