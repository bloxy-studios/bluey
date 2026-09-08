# Google Gemini Developer API — state of the world as of 2026-09-08

Scope: the Google AI Studio single-API-key path (`generativelanguage.googleapis.com`), NOT Vertex AI / "Gemini Enterprise Agent Platform". Everything below was read from live pages on 2026-09-08 (see section H). Anything not confirmed on an official page is marked **UNVERIFIED**.

Target use: migrate a TypeScript (Vite + React, Bun tooling) + Rust (Tauri) desktop app to Gemini for (1) streaming chat, (2) screenshot understanding, (3) batch + real-time audio transcription, (4) embeddings.

## TL;DR — the four decisions

| Feature | Use this model ID | API surface | Key limit |
|---|---|---|---|
| Chat / LLM answers (streaming) | `gemini-3.8-flash` (GA 2026-09-02; intro price $0.75/$3.75 per 1M until 2026-12-31). Cheaper fallback: `gemini-3.5-flash-lite` ($0.30/$2.50) | Interactions API (`ai.interactions.create({... stream: true})`) or legacy `ai.models.generateContentStream` — both fully supported | 1,048,576 in / 65,536 out; thinking `low|medium|high` (default `medium`; `minimal` returns an error on 3.7/3.8) |
| Vision (screenshots) | same `gemini-3.8-flash` | inline base64 `image/png|jpeg|webp` part; per-item `resolution: "high"` (1,120 tokens/image) | inline request ≤ 20 MB per the image guide (Interactions "file input methods" page says 100 MB) — use Files API above 20 MB |
| Batch transcription | `gemini-3.5-transcribe` (GA 2026-08-26) | `generateContent` with `generationConfig.audioTranscriptionConfig` **or** Interactions `generation_config.transcription_config` | ≤ 1 h audio/request (≤ 30 min with diarization or word timestamps); ~$0.005/min |
| Real-time transcription (mic / system audio) | `gemini-3.5-transcribe-live` (GA 2026-08-26) | Live API WebSocket (`ai.live.connect`, `responseModalities: [TEXT]`, `inputAudioTranscription`) | raw 16-bit PCM 16 kHz mono LE, `audio/pcm;rate=16000`, ~100 ms chunks; **10-minute session cap**; interim + final events; ~$0.009/min |
| Embeddings | `gemini-embedding-2` (GA 2026-04-22; multimodal; $0.20/1M text) — or `gemini-embedding-001` (text-only, shuts down 2028-05-14) | `ai.models.embedContent` / `POST models/{m}:embedContent` | 3072 dims default, MRL-truncate to 768/1536 (auto-normalized on embedding-2); 8,192 input tokens; no `taskType` on embedding-2 (use prompt prefixes) |
| SDK | `@google/genai` **2.21.0** (released 2026-09-02); Node ≥ 20 (v3 will require Node 22 — pin `<3.0.0`); browser build `dist/web/index.mjs` | `import { GoogleGenAI } from '@google/genai'` | Old `@google/generative-ai` deprecated since 2025-11-30 |

Two surprises worth internalising before writing code:

1. **Google's docs have pivoted to the "Interactions API"** (`POST /v1beta/interactions`, `ai.interactions.create`). It has been GA since June 2026 and is "recommended for all new projects"; `generateContent` is now labelled *legacy* but "remains fully supported" and still gets all models (the docs have a toggle; the legacy pages live under `/gemini-api/docs/generate-content/*`). New features launch on Interactions first. Batch API, explicit caching, custom safety settings and (Python) automatic function calling are only on `generateContent`.
2. **Sampling parameters are deprecated for Gemini 3.x.** `temperature`, `top_p`, `top_k` were deprecated in the 2026-07-21 release notes; Google says strip them and leave the default 1.0. `thinking_budget` is replaced by `thinking_level`; `candidate_count` is unsupported on 3.x.

---

## (A) Model catalog

All prices are Paid-tier **Standard** per 1M tokens (USD); Batch and Flex are 50% of standard; Priority is 1.8x. "Free" = model appears with "Free of charge" in the free-tier column on the pricing page. Context/output limits and capability flags come from the individual model pages; release/shutdown dates from the deprecations page and release notes.

| Model ID | Status | Released | Context in / out | Inputs → outputs | Thinking | Live API | Price in / out (std) | Free tier | Shutdown | Notes |
|---|---|---|---|---|---|---|---|---|---|---|
| `gemini-3.8-flash` | GA stable | 2026-09-02 | 1,048,576 / 65,536 | text, image, video, audio, PDF → text | `low`,`medium`(default),`high`; **`minimal` returns an error** | No | $0.75 / $3.75 through 2026-12-31, then $1.50 / $7.50. Cache read $0.075→$0.15; cache storage $0.50→$1.00 per 1M tok/h | Yes | none announced | Caching, code exec, computer use (preview), file search, function calling, Maps+Search grounding, structured outputs, URL context, Batch/Flex/Priority. Default model for Managed Agents. "Can use more tokens on long tasks by design". Knowledge cutoff **UNVERIFIED** (not on model card) |
| `gemini-3.7-flash` | GA stable | 2026-08-13 | 1,048,576 / 65,536 | same | `low`,`medium`(default),`high`; `minimal` errors | No | same intro pricing as 3.8 ($0.75/$3.75 → $1.50/$7.50) | Yes | none | Same capability set as 3.8 |
| `gemini-3.6-flash` | GA stable | 2026-07-21 | 1,048,576 / 65,536 | same | `minimal`,`low`,`medium`(default),`high` | No | $0.75 / $3.75 → $1.50 / $7.50 (same intro schedule) | Yes | none | "improved token efficiency" vs 3.5 |
| `gemini-3.5-flash` | GA stable | 2026-05-19 | 1,048,576 / 65,536 | same | `minimal`,`low`,`medium`(default),`high` | No | $1.50 / $9.00 (no intro discount; cache $0.15 + $1.00/h) | Yes | none | Knowledge cutoff Jan 2025. Was made the target of `gemini-flash-latest` on 2026-05-19 |
| `gemini-3.5-flash-lite` | GA stable | 2026-07-21 | 1,048,576 / 65,536 | same | `minimal`(default),`low`,`medium`,`high` | No | $0.30 (all modalities) / $2.50; cache $0.03 (**paid only**) | Yes (no caching) | none | Cheapest 3.5-gen option; 500 RPD free per third-party measurements |
| `gemini-3.1-flash-lite` | GA stable | 2026-05-07 | 1,048,576 / 65,536 | same | `minimal`(default),`low`,`medium`,`high` | No | $0.25 text/img/video, $0.50 audio / $1.50 | Yes | **2027-05-07** → `gemini-3.5-flash-lite` | Long-term stable per Google; computer use not supported |
| `gemini-3-flash-preview` | Preview | 2025-12-17 | 1,048,576 / 65,536 | same | `minimal`,`low`,`medium`,`high`(default) | No | $0.50 text/img/video, $1.00 audio / $3.00 | Yes | none announced; recommended replacement `gemini-3.6-flash` | Knowledge cutoff Jan 2025 |
| `gemini-3.1-pro-preview` (+ `-customtools`) | Preview | 2026-02-19 | 1,048,576 / 65,536 | same | `low`,`medium`,`high`(default) | No | $2 / $12 (≤200k), $4 / $18 (>200k) | **No** | none | No free tier in API |
| `gemini-2.5-flash` | GA stable | 2025-06-17 | 1,048,576 / 65,536 | text, image, video, audio → text | `thinkingBudget` 0–24,576 (-1 dynamic) | No | $0.30 ($1.00 audio) / $2.50 | Yes | none | Knowledge cutoff Jan 2025; Google's own Rust crate README claims "unavailable to newly created API keys" (**UNVERIFIED**) |
| `gemini-2.5-flash-lite` | GA stable | 2025-07-22 | — | — | budget 512–24,576; off by default | No | $0.10 ($0.30 audio) / $0.40 | Yes | none | |
| `gemini-2.5-pro` | GA stable | 2025-06-17 | — | — | budget 128–32,768 (cannot disable) | No | $1.25 / $10 (≤200k) | Yes | none | |
| `gemini-3.5-transcribe` | GA stable | 2026-08-26 | audio ≤ 1 h/request (≤ 30 min with diarization or word timestamps) | audio → text + word annotations | none | No (unary only) | $2.00/1M audio-in (≈$0.003/min at 25 tok/s); $12.00/1M text-out (≈$0.002/min) ⇒ ≈ **$0.005/min** | Yes | none | 85+ languages auto-detect, code-switching, diarization ≤ 8 speakers (3+ experimental), word timestamps, custom vocabulary ≤ 1,000 terms, `SMART` mode. No caching/batch/flex/priority/thinking/tools |
| `gemini-3.5-transcribe-live` | GA stable | 2026-08-26 ("August 2026") | 10 min per session | PCM audio stream → interim + final text | none | **Yes (Live API only)** | $3.50/1M audio-in (≈$0.005/min); $21.00/1M text-out (≈$0.004/min) ⇒ ≈ **$0.009/min** | Yes | none | No diarization, no word timestamps over Live; custom vocabulary and `SMART` mode supported; VAD auto / hybrid / manual |
| `gemini-3.1-flash-live-preview` | Preview | 2026-03-11 (deprecations page) / announced 2026-03-26 | 131,072 / 65,536 | text, image, audio, video → text + audio | `thinkingLevel` `minimal`(default),`low`,`medium`,`high` | **Yes** | in: $0.75 text, $3.00 audio (≈$0.005/min), $1.00 image/video; out: $4.50 text, $12.00 audio (≈$0.018/min) | Yes | none | Current recommended Live model. Function calling synchronous only; no proactive audio / affective dialog; one server event may carry several parts |
| `gemini-2.5-flash-native-audio-preview-12-2025` | Preview | 2025-12-12 | 131,072 / 8,192 | audio, video, text → audio + text | `thinkingBudget` | Yes | in: $0.50 text, $3.00 audio/video; out: $2.00 text, $12.00 audio | Yes | none; replacement `gemini-3.1-flash-live-preview` | Supports async function calling, proactive audio, affective dialog |
| `gemini-3.5-live-translate-preview` | Preview | June 2026 | — | speech → translated speech | — | Yes | $3.50 / $21.00 | Yes | none | Not needed here |
| `gemini-embedding-2` | GA stable | 2026-04-22 | 8,192 input tokens; dims 128–3072 (recommended 768/1536/3072) | text, image (≤6, PNG/JPEG), audio (≤180 s, MP3/WAV), video (≤120 s), PDF (1 file ≤ 6 pages) → embedding | — | — | text $0.20; image $0.45 ($0.00012/img); audio $6.50 ($0.00016/s); video $12.00. Batch 50% | Yes | none | Auto-normalizes truncated dims. **No `task_type`** — put `task: search result \| query: ...` prefixes in the text. Multiple parts in one `contents` ⇒ ONE aggregated embedding; wrap each in a `Content` for separate vectors |
| `gemini-embedding-001` | GA stable | 2025-07-14 | 2,048 input tokens; dims 128–3072 | text → embedding | — | — | $0.15 (batch $0.075) | Yes | **2028-05-14** → `gemini-embedding-2` | Supports `taskType` (SEMANTIC_SIMILARITY, RETRIEVAL_DOCUMENT/QUERY, CODE_RETRIEVAL_QUERY, QUESTION_ANSWERING, FACT_VERIFICATION, CLASSIFICATION, CLUSTERING); manual L2-normalize if dims < 3072. Embedding spaces of 001 and 2 are incompatible |
| `gemini-embedding-2-preview` | shut down 2026-08-10 | 2026-03-10 | | | | | | | shut down | Still referenced in the OpenAI-compat page and the models list (stale docs) |
| `gemini-omni-1.1-flash` | GA | 2026-08-27 | video gen | | | | $1.50 in / $9.00 text, $17.50 video out | No | preview endpoint `gemini-omni-flash-preview` deprecated 2026-09-30 | Not relevant |

Already shut down (do not use): `gemini-2.0-flash`, `gemini-2.0-flash-001`, `gemini-2.0-flash-lite`, `gemini-2.0-flash-lite-001` (2026-06-01); `gemini-3-pro-preview` (2026-03-09, ID now redirects to `gemini-3.1-pro-preview`); `gemini-3.1-flash-lite-preview` (2026-05-25); `text-embedding-004` (2026-01-14); `embedding-001`, `embedding-gecko-001`, `gemini-embedding-exp*` (2025-10-30); Live models `gemini-2.0-flash-live-001`, `gemini-live-2.5-flash-preview` (2025-12-09); all 1.5 models (2025-09-29). Upcoming: `gemini-2.5-flash-image` shuts down 2026-10-02; `gemini-robotics-er-1.6-preview` 2026-08-31.

Model aliases: `gemini-flash-latest`, `gemini-flash-lite-latest`, `gemini-pro-latest` exist and are "hot-swapped with every new release of a specific model variation" (2-week email notice for breaking changes). The changelog records `gemini-flash-latest` → `gemini-3-flash-preview` (2026-01-21) and → `gemini-3.5-flash` (2026-05-19); no later move is recorded, so its current target is **UNVERIFIED**. Google: "Most production apps should use a specific stable model." There is no `gemini-3.8-flash-preview`; 3.8 shipped straight to a stable ID. Version-pattern note on the models page still says the convention is "as of September 2025".

Knowledge cutoff: official pages state January 2025 for the Gemini 3 family, Gemini 3.5 Flash, 2.5 Flash and 2.5 Flash Live. The 3.6/3.7/3.8 model cards carry no cutoff field (**UNVERIFIED**; a third-party article claims March 2026 for 3.8).

## (B) Free tier & rate limits

What the official rate-limits page says (it **no longer publishes per-model RPM/TPM/RPD tables**; "View your active rate limits in AI Studio" at https://aistudio.google.com/rate-limit):

- Limits are per **project** (not per key), on three axes: RPM, TPM (input tokens/min), RPD (resets midnight Pacific). Preview/experimental models are more restricted. Exceeding any axis ⇒ `429 RESOURCE_EXHAUSTED`.
- Usage tiers: Free (active project) → Tier 1 (link a billing account; instant; billing-tier cap $250) → Tier 2 ($100 paid + 3 days; cap $2,000) → Tier 3 ($1,000 paid + 30 days; cap $20k–$100k+). Upgrades after Tier 1 take effect within ~10 minutes.
- Spend-based limits per rolling 10 minutes: Tier 1 $10, Tier 2 $50, Tier 3 $200 (also 429).
- Priority inference: 0.3x the standard limit. Batch API: 100 concurrent jobs, 2 GB input file, 20 GB storage; enqueued-token caps at Tier 1: 3.8/3.7/3.6/3.5 Flash 3M, 3.5 Flash-Lite 10M, Gemini Embedding 500k (Tier 2: 400M / 500M / 5M; Tier 3: 1B / 1B / 10M).

What the free tier of an AI Studio key includes (pricing page "Free Tier" columns): free input/output on `gemini-3.8-flash`, `3.7-flash`, `3.6-flash`, `3.5-flash`, `3.5-flash-lite`, `3.1-flash-lite`, `3-flash-preview`, `gemini-3.5-transcribe`, `gemini-3.5-transcribe-live`, `gemini-3.1-flash-live-preview`, `gemini-2.5-flash-native-audio-preview-12-2025`, 2.5 Flash/Flash-Lite/Pro, `gemini-embedding-2`, `gemini-embedding-001`, TTS previews, Gemma 4. **Not** on the free tier: `gemini-3.1-pro-preview`, image generation (Nano Banana 2 / Pro), Omni, Veo, Lyria, Computer Use 2.5. Free-tier content "used to improve our products"; Batch API "Not available"; Grounding "Not available" for Gemini 3 models on free (testable in AI Studio); context caching free on 3.x Flash but "Not available" on 3.5-flash-lite/3.1-flash-lite/2.5. Interactions API stores requests for 1 day on free vs 55 days paid (opt out with `store: false`).

Per-model free-tier numbers — **UNVERIFIED (third-party measurements, not official)**: a dev.to measurement on 2026-09-02 and a ScriptByAI article on 2026-09-07 both report Free tier = **5 RPM / 20 RPD** for `gemini-3.7-flash`, `3.6-flash`, `3.5-flash`, `3-flash-preview` (a Google forum thread from 2026-09-03 complains 3.8 Flash is "20 RPD"), **15 RPM / 500 RPD** for `gemini-3.5-flash-lite` and `3.1-flash-lite`, and **100 RPM / 1,000 RPD** for `gemini-embedding-2`. The 429 body reportedly carries a `google.rpc.QuotaFailure` with `quotaId` such as `GenerateRequestsPerDayPerProjectPerModel-FreeTier` and `quotaValue`. Treat Free tier as unusable for a shipping desktop app; Tier 1 (link billing, no spend required) removes the daily caps. Transcribe-model free limits: not found anywhere — **UNVERIFIED**.

API-key policy change (api-key page, verbatim gist): new AI Studio keys are "auth keys" bound to a service account; "The Gemini API rejects requests from unrestricted standard keys"; "On September 2026: the Gemini API will reject requests from Standard keys. You must migrate to auth keys before this date." Client SDKs read `GEMINI_API_KEY` or `GOOGLE_API_KEY` (the latter wins if both set). "Never expose keys client-side in production ... run a backend proxy server." Ephemeral tokens are the sanctioned client-side mechanism, but **only for the Live API**.

## (C) `@google/genai` SDK reference

Facts (js-genai `package.json`, README, CHANGELOG, typedoc, all read 2026-09-08):

- Current release **2.21.0** (2026-09-02, "Add Gemini 3.8 Flash model to SDKs"). Recent: 2.20.0 (2026-08-31, `audio/webm` MIME), 2.19.0 (2026-08-25, `AudioTranscriptionConfigMode`), 2.18.0 (2026-08-19, `mode` enum `VERBATIM|SMART`).
- `engines.node: ">=20.0.0"`; README warns v3.0.0 will require Node 22 and remove `LiveConnectConfig.generation_config` (set fields directly on `LiveConnectConfig`), `GenerationConfigThinkingConfig` (use `ThinkingConfig`), and AFC from direct `Models.generateContent` calls (use `Chats`). Google: "pin the SDK version to `< 3.0.0`".
- Entry points: `main dist/node/index.mjs`, `browser dist/web/index.mjs`; exports `.`, `./web`, `./node`, `./tokenizer`. Runs in the browser; initialization is identical, with the CAUTION "Avoid exposing API keys in client-side code." Deps: `google-auth-library`, `p-retry`, `protobufjs`, `ws` (Node Live); peer dep `@modelcontextprotocol/sdk`. Bun compatibility is not documented anywhere — **UNVERIFIED** (Bun ≥ 1.x generally runs Node ≥ 20 ESM; test `ws` + Live).
- Interactions API needs SDK ≥ 2.3.0; SDK 2.0.0 introduced the breaking `outputs` → `steps` schema (May 2026). Google's "What's new in 3.5" says "update to google-genai SDK v2.0.0 or later".
- Client: `new GoogleGenAI({ apiKey })` (or `{}` to read `GEMINI_API_KEY`/`GOOGLE_API_KEY` in Node). Default API version is `v1beta`; `apiVersion: 'v1'` selects stable endpoints (not needed; Live/ephemeral tokens require v1beta).
- Submodules: `ai.models` (generateContent, generateContentStream, embedContent, countTokens), `ai.interactions` (create/get/delete/cancel), `ai.chats`, `ai.files`, `ai.caches`, `ai.live`, `ai.authTokens`, `ai.operations`.
- Per-request cancel: `config.abortSignal?: AbortSignal` — "client-only operation... will not cancel the request in the service. You will still be charged" (typedoc GenerateContentConfig).
- HTTP options: `httpOptions: { apiVersion, baseUrl, headers, timeout /* ms, per-attempt deadline */, retryOptions: { attempts /* default 5 incl. original */, initialDelay /* 1.0 s */, maxDelay /* 60 s */, expBase /* 2 */, jitter /* 1 */, httpStatusCodes /* default 408, 429, 5xx */ }, extraBody }` — settable on the client and per request (CHANGELOG: "Support per-request retryOptions"; "Make HttpOptions.timeout a per-attempt deadline").
- Errors: `ApiError extends Error` with `name`, `message`, `status` (HTTP code). Codegen instructions: "Incorrect `GoogleGenAIError` -> Correct `ApiError`".
- Helpers seen in official docs: `createUserContent`, `createPartFromUri`, `Type` enum (OBJECT/STRING/...), `ThinkingLevel`, `Modality`, `MediaResolution`, `StartSensitivity`, `EndSensitivity`, `FunctionCallingConfigMode`, `mcpToTool`. `createPartFromBase64` — not seen in any fetched page: **UNVERIFIED** (an `{ inlineData: { mimeType, data } }` literal works regardless).
- Interactions JS naming: official pages use snake_case keys (`generation_config`, `previous_interaction_id`, `system_instruction`, `response_format`, `output_text`, `event_type`, `mime_type`) while the "What's new in 3.5" page uses camelCase (`generationConfig: { thinkingLevel }`, `previousInteractionId`). The SDK appears to accept both; **verify against `@google/genai` types** before relying on one.

Google's own codegen rules (js-genai/codegen_instructions.md) worth pasting into the implementation prompt: never `getGenerativeModel`, never a separate `generationConfig` object (pass `config: {...}`), `GenerateContentResponse`/`GenerateContentParameters` are the type names, `response.text` is the shorthand, `ApiError` is the error class. (Its model recommendations are stale — it still names `gemini-3-flash-preview`.)

### C.1 Streaming chat with system instruction + thinking level

Legacy generateContent path (verbatim from the legacy Text generation and Thinking guides; three separate official samples):

```javascript
import { GoogleGenAI, ThinkingLevel } from "@google/genai";

const ai = new GoogleGenAI({});

async function main() {
  const response = await ai.models.generateContent({
    model: "gemini-3.8-flash",
    contents: "How does AI work?",
    config: {
      thinkingConfig: {
        thinkingLevel: ThinkingLevel.LOW,
      },
    }
  });
  console.log(response.text);
}

await main();
```

```javascript
import { GoogleGenAI } from "@google/genai";

const ai = new GoogleGenAI({});

async function main() {
  const response = await ai.models.generateContent({
    model: "gemini-3.8-flash",
    contents: "Hello there",
    config: {
      systemInstruction: "You are a cat. Your name is Neko.",
    },
  });
  console.log(response.text);
}

await main();
```

```javascript
import { GoogleGenAI } from "@google/genai";

const ai = new GoogleGenAI({});

async function main() {
  const response = await ai.models.generateContentStream({
    model: "gemini-3.8-flash",
    contents: "Explain how AI works",
  });

  for await (const chunk of response) {
    console.log(chunk.text);
  }
}

await main();
```

Multi-turn with the SDK chat helper (verbatim, legacy guide) — history is resent each turn; the SDK preserves thought signatures automatically:

```javascript
import { GoogleGenAI } from "@google/genai";

const ai = new GoogleGenAI({});

async function main() {
  const chat = ai.chats.create({
    model: "gemini-3.8-flash",
    history: [
      {
        role: "user",
        parts: [{ text: "Hello" }],
      },
      {
        role: "model",
        parts: [{ text: "Great to meet you. What would you like to know?" }],
      },
    ],
  });

  const stream1 = await chat.sendMessageStream({
    message: "I have 2 dogs in my house.",
  });
  for await (const chunk of stream1) {
    console.log(chunk.text);
    console.log("_".repeat(80));
  }

  const stream2 = await chat.sendMessageStream({
    message: "How many paws are in my house?",
  });
  for await (const chunk of stream2) {
    console.log(chunk.text);
    console.log("_".repeat(80));
  }
}

await main();
```

Composed (NOT verbatim — combines the three official shapes above; `config` fields are all documented on `GenerateContentConfig`):

```typescript
const stream = await ai.models.generateContentStream({
  model: "gemini-3.8-flash",
  contents: history, // Content[] with role "user" | "model"; keep every part incl. thoughtSignature
  config: {
    systemInstruction: SYSTEM_PROMPT,
    thinkingConfig: { thinkingLevel: ThinkingLevel.LOW }, // low | medium (default) | high on 3.8
    abortSignal: controller.signal,
    httpOptions: { timeout: 60_000, retryOptions: { attempts: 3 } },
  },
});
for await (const chunk of stream) { if (chunk.text) onDelta(chunk.text); }
```

Interactions API streaming (verbatim, Text generation guide). Server-side state via `previous_interaction_id`; per-interaction params (`tools`, `system_instruction`, `generation_config`) must be re-sent every turn:

```javascript
import { GoogleGenAI } from "@google/genai";

const ai = new GoogleGenAI({});

async function main() {
  const stream = await ai.interactions.create({
    model: "gemini-3.8-flash",
    input: "Explain how AI works",
    stream: true,
  });

  for await (const event of stream) {
    if (event.event_type === "step.delta") {
      if (event.delta.type === "text") {
        process.stdout.write(event.delta.text);
      }
    }
  }
}

await main();
```

```javascript
import { GoogleGenAI } from "@google/genai";

const ai = new GoogleGenAI({});

async function main() {
  const interaction = await ai.interactions.create({
    model: "gemini-3.8-flash",
    input: "Hello there",
    system_instruction: "You are a cat. Your name is Neko.",
  });
  console.log(interaction.output_text);
}

await main();
```

```javascript
import { GoogleGenAI } from "@google/genai";

const ai = new GoogleGenAI({});

async function main() {
  const interaction = await ai.interactions.create({
    model: "gemini-3.8-flash",
    input: "How does AI work?",
    generation_config: {
      thinking_level: "low",
    },
  });
  console.log(interaction.output_text);
}

await main();
```

SSE event flow for Interactions streaming: `interaction.created` → per step `step.start` / `step.delta`* / `step.stop` (step types `thought`, `model_output`, `function_call`, ...) → `interaction.completed` (with `usage.total_thought_tokens` etc.) → `done` `[DONE]`. Delta types include `text`, `thought_summary`, `thought_signature`. Errors arrive as `event_type: "error"` with `{code, message}`. Stateless mode: `store: false` and resend all returned `steps` (thought steps carry signatures).

### C.2 Image understanding with inline base64 (screenshots)

Legacy generateContent (verbatim, legacy Image understanding guide):

```javascript
import { GoogleGenAI } from "@google/genai";
import * as fs from "node:fs";

const ai = new GoogleGenAI({});
const base64ImageFile = fs.readFileSync("path/to/small-sample.jpg", {
  encoding: "base64",
});

const contents = [
  {
    inlineData: {
      mimeType: "image/jpeg",
      data: base64ImageFile,
    },
  },
  { text: "Caption this image." },
];

const response = await ai.models.generateContent({
  model: "gemini-3.8-flash",
  contents: contents,
});
console.log(response.text);
```

Interactions API with per-item media resolution (verbatim, Image understanding + Media resolution guides):

```javascript
import { GoogleGenAI } from "@google/genai";
import * as fs from "node:fs";

const client = new GoogleGenAI({});
const base64ImageFile = fs.readFileSync("path/to/small-sample.jpg", {
  encoding: "base64",
});

const interaction = await client.interactions.create({
    model: "gemini-3.8-flash",
    input: [
        {type: "text", text: "Caption this image."},
        {
            type: "image",
            data: base64ImageFile,
            mime_type: "image/jpeg"
        }
    ]
});
console.log(interaction.output_text);
```

```javascript
  const interaction = await ai.interactions.create({
    model: "gemini-3.8-flash",
    input: [
      { type: "text", text: "Describe this image:" },
      {
        type: "image",
        uri: myfile.uri,
        mime_type: myfile.mimeType,
        resolution: "high"
      }
    ],
  });
```

Facts: supported image MIME types `image/png`, `image/jpeg`, `image/webp`, `image/heic`, `image/heif` (Interactions `ImageContent` enum additionally lists gif/bmp/tiff). Max 3,600 images per request. "Inline image data limits your total request size (text prompts, system instructions, and inline bytes) to 20MB" (image guide) — the newer File-input-methods page says "100 MB per request or payload (50 MB for PDFs)" (raised from 20 MB on 2026-01-08 per changelog); use the Files API above 20 MB to be safe. Gemini 3 token cost per image by `media_resolution`: unspecified/default 1,120, `low` 280, `medium` 560, `high` 1,120, `ultra_high` 2,240 (per-item only; needed for computer use). Google's recommendation for images: `high`. Legacy (2.x) accounting: 258 tokens if ≤ 384 px each side, otherwise 768×768 tiles × 258. Image order tip: docs disagree (Interactions guide: text before image; legacy guide: image before text) — either works. Object detection returns `box_2d` `[ymin, xmin, ymax, xmax]` normalized 0–1000; segmentation masks are **not supported on Gemini 3.x** (use 2.5 Flash with thinking off).

### C.3 Batch audio transcription with `gemini-3.5-transcribe`

Two documented request shapes. The legacy generateContent path (verbatim, legacy Audio transcription guide) is the simplest for a Rust/reqwest backend too:

```javascript
import { GoogleGenAI } from "@google/genai";

const ai = new GoogleGenAI({});

const audioFile = await ai.files.upload({
  file: "path/to/sample.mp3",
  mimeType: "audio/mp3",
});

const response = await ai.models.generateContent({
  model: "gemini-3.5-transcribe",
  contents: [audioFile],
});

console.log(response.text);
```

```javascript
const response = await ai.models.generateContent({
  model: "gemini-3.5-transcribe",
  contents: [audioFile],
  config: {
    audioTranscriptionConfig: {
      languageCodes: [],
    },
  },
});
```

```javascript
const response = await ai.models.generateContent({
  model: "gemini-3.5-transcribe",
  contents: [audioFile],
  config: {
    audioTranscriptionConfig: {
      customVocabulary: ["Gemini", "Kubernetes", "BigQuery"],
    },
  },
});
```

```javascript
const config = {
  audioTranscriptionConfig: {
    diarization: true,
    wordTimestamp: true,
  },
};
```

```javascript
const response = await ai.models.generateContent({
  model: "gemini-3.5-transcribe",
  contents: [audioFile],
  config: {
    audioTranscriptionConfig: {
      mode: "SMART",
    },
  },
});
console.log(response.text);
```

```javascript
function extractWordTranscriptions(response) {
  const words = [];
  for (const candidate of response.candidates ?? []) {
    for (const part of candidate.content?.parts ?? []) {
      const transcription = part.audioTranscription;
      if (transcription) {
        const speaker = transcription.speakerLabel ?? "";
        for (const wordInfo of transcription.words ?? []) {
          words.push({
            word: wordInfo.word ?? "",
            speaker: speaker,
            startOffset: wordInfo.startOffset ?? "",
            endOffset: wordInfo.endOffset ?? "",
          });
        }
      }
    }
  }
  return words;
}
```

generateContent response shape with diarization/timestamps (verbatim REST example):

```json
{
  "candidates": [
    {
      "content": {
        "parts": [
          {
            "audioTranscription": {
              "speakerLabel": "spk_1",
              "words": [
                { "word": "Hello", "startOffset": "0.100s", "endOffset": "0.450s" },
                { "word": "world", "startOffset": "0.500s", "endOffset": "0.850s" }
              ]
            }
          }
        ],
        "role": "model"
      },
      "finishReason": "STOP"
    }
  ]
}
```

Interactions API path (verbatim, Audio transcription guide) — note the different config nesting (`mode` is either the string `"smart"` or an object `{type: "verbatim", diarization_mode, timestamp_granularities}`):

```javascript
import { GoogleGenAI } from "@google/genai";

const client = new GoogleGenAI({});

const audioFile = await client.files.upload({
  file: "path/to/sample.mp3",
  config: { mime_type: "audio/mp3" },
});

const interaction = await client.interactions.create({
  model: "gemini-3.5-transcribe",
  input: [
    {
      type: "audio",
      uri: audioFile.uri,
      mime_type: audioFile.mimeType,
    },
  ],
});

console.log(interaction.output_text);
```

```javascript
const interaction = await client.interactions.create({
  model: "gemini-3.5-transcribe",
  input: [
    {
      type: "audio",
      uri: audioFile.uri,
      mime_type: audioFile.mimeType,
    },
  ],
  generation_config: {
    transcription_config: {
      mode: {
        type: "verbatim",
        diarization_mode: "speaker",
        timestamp_granularities: ["word"],
      },
    },
  },
});
```

```javascript
function extractWordAnnotations(interaction) {
  const words = [];
  for (const step of interaction.steps ?? []) {
    for (const content of step.content ?? []) {
      for (const annotation of content.annotations ?? []) {
        if (annotation.type === "word_info") {
          words.push(annotation);
        }
      }
    }
  }
  return words;
}
```

Inline audio instead of Files API: the Interactions `AudioContent` type has `data` (base64), `mime_type`, `uri`, plus optional `sample_rate` and `channels` (useful for raw `audio/l16` PCM); the Audio-understanding guide shows `{ type: "audio", data: <base64>, mime_type: "audio/mp3" }` for "small audio files under 20MB total request size". For generateContent use `{ inlineData: { mimeType, data } }`. The transcribe guide's best practice: "For files longer than a few seconds, upload the file using `client.files.upload`".

Transcribe facts: MIME types `audio/wav`, `audio/mp3`, `audio/aiff`, `audio/aac`, `audio/ogg`, `audio/flac`, `audio/mpeg`, `audio/m4a`, `audio/l16`, `audio/opus`, `audio/alaw`, `audio/mulaw`, `audio/webm`. Limits: 1 h per request; 30 min when diarization or word timestamps enabled; diarization ≤ 8 speakers (3+ experimental, labels `spk_1`, `spk_2`, ...); word timestamps "may degrade overall transcription accuracy"; `custom_vocabulary` ≤ 1,000 terms (best ≤ 100) and is **rejected** when combined with diarization or timestamps; `SMART` mode cannot be combined with timestamps or diarization; `language_codes` empty/omitted ⇒ auto-detect with code-switching (BCP-47 list of ~85 locales on the guide). `languageAuto`/`languageHints`/`adaptationPhrases` fields are deprecated in the API reference — use `languageCodes`/`customVocabulary`. Billing: 25 audio tokens/s input, ~175 text tokens/min output.

Alternative for "understanding" (summaries, Q&A over audio): `gemini-3.8-flash` with an audio part — 32 tokens/s, up to 9.5 h per prompt, audio downsampled to 16 kbps mono.

### C.4 Live streaming transcription with `gemini-3.5-transcribe-live`

All verbatim from the Live transcription guide.

```javascript
import { GoogleGenAI, Modality } from '@google/genai';

const ai = new GoogleGenAI({});
const model = 'gemini-3.5-transcribe-live';

const config = {
  responseModalities: [Modality.TEXT],
  inputAudioTranscription: {
    languageCodes: [], // Automatic language detection
  },
};

async function main() {
  const session = await ai.live.connect({
    model: model,
    config: config,
    callbacks: {
      onopen: () => console.log('Connected to Live Transcription'),
      onmessage: (message) => {
        const content = message.serverContent;
        if (content?.inputTranscription) {
          console.log('Transcript:', content.inputTranscription.text);
        }
      },
      onerror: (e) => console.error('Error:', e.message),
      onclose: (e) => console.log('Connection closed:', e.reason),
    },
  });
}

main();
```

Interim vs final events:

```javascript
onmessage: (message) => {
  const content = message.serverContent;
  if (!content) return;

  if (content.interimInputTranscription) {
    // Update live subtitle preview on screen
    renderInterimPreview(content.interimInputTranscription.text);
  }

  if (content.inputTranscription) {
    // Append final committed transcript to chat history
    commitFinalTranscript(content.inputTranscription.text);
  }
};
```

Sending audio ("Raw 16-bit PCM at 16kHz (mono, little-endian)"; "Send audio in chunks of 100ms (1,024 to 2,048 frames)"; MIME `audio/pcm;rate=16000` or the matching sample rate — the Live API resamples any rate you declare):

```javascript
// Send base64-encoded PCM audio chunk
session.sendRealtimeInput({
  audio: {
    data: audioChunkBase64,
    mimeType: 'audio/pcm;rate=16000'
  }
});

// Signal stream end
session.sendRealtimeInput({
  audioStreamEnd: true
});
```

Custom vocabulary and Smart mode:

```javascript
const config = {
  responseModalities: [Modality.TEXT],
  inputAudioTranscription: {
    languageCodes: [],
    customVocabulary: ['Gemini', 'Kubernetes', 'BigQuery'],
  },
};
```

```javascript
const config = {
  responseModalities: [Modality.TEXT],
  inputAudioTranscription: {
    mode: 'SMART',
  },
};
```

VAD strategies. Automatic (default, server-side). Hybrid: keep auto VAD and send `audioStreamEnd: true` when your local VAD detects silence for immediate finalization (server VAD remains the fallback). Manual push-to-talk:

```javascript
const config = {
  responseModalities: [Modality.TEXT],
  realtimeInputConfig: {
    automaticActivityDetection: {
      disabled: true,
    },
  },
  inputAudioTranscription: {},
};

// Signal speech start
session.sendRealtimeInput({ activityStart: {} });

// Stream audio...

// Signal speech end
session.sendRealtimeInput({ activityEnd: {} });
```

Raw WebSocket variant (verbatim) — the exact URL:

```javascript
const API_KEY = "YOUR_API_KEY";
const MODEL_NAME = "gemini-3.5-transcribe-live";
const WS_URL = `wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent?key=${API_KEY}`;

const websocket = new WebSocket(WS_URL);

websocket.onopen = () => {
  console.log('WebSocket connected');

  const setupMessage = {
    setup: {
      model: `models/${MODEL_NAME}`,
      generationConfig: {
        responseModalities: ['TEXT'],
      },
      inputAudioTranscription: {
        languageCodes: []
      }
    }
  };
  websocket.send(JSON.stringify(setupMessage));
};

websocket.onmessage = (event) => {
  const response = JSON.parse(event.data);
  const content = response.serverContent;
  if (content?.inputTranscription) {
    console.log('Transcript:', content.inputTranscription.text);
  }
};
```

```javascript
// Send base64-encoded PCM audio chunk
websocket.send(JSON.stringify({
  realtimeInput: {
    audio: {
      data: audioChunkBase64,
      mimeType: 'audio/pcm;rate=16000'
    }
  }
}));

// Signal stream end
websocket.send(JSON.stringify({
  realtimeInput: {
    audioStreamEnd: true
  }
}));
```

Ephemeral token constrained to the transcribe model (verbatim; mint on your trusted side, hand `token.name` to the WebView):

```javascript
import { GoogleGenAI } from '@google/genai';

const client = new GoogleGenAI({});
const expireTime = new Date(Date.now() + 30 * 60 * 1000).toISOString();

const token = await client.authTokens.create({
  config: {
    uses: 1,
    expireTime: expireTime,
    liveConnectConstraints: {
      model: 'gemini-3.5-transcribe-live',
      config: {
        responseModalities: ['TEXT'],
        inputAudioTranscription: {
          languageCodes: [],
        },
      },
    },
  },
});
```

Live-transcribe limits (verbatim list): "Live transcription sessions support continuous streaming for up to 10 minutes"; speaker diarization not supported; word-level timestamps not supported ("emits utterance-level timestamps"); custom vocabulary ≤ 1,000 terms; SMART mode "cannot be combined with word annotations". Server fields: `serverContent.interimInputTranscription` (speculative, frequent) and `serverContent.inputTranscription` (final, authoritative; each `BidiGenerateContentTranscription` has `text` and `languageCode`). Implication for a meeting-length recorder: reconnect every ≤ 10 min (session resumption is a general Live feature — whether `sessionResumption` applies to the transcribe model is **UNVERIFIED**), and treat `GoAway.timeLeft` as the reconnect cue. VAD tuning guidance from the capabilities guide: `silenceDurationMs` 500–800 ms (server default ≈ 800 ms); too low fragments utterances; manual VAD bypasses the server's prefix buffer, so use ≥ 500 ms end-of-speech threshold client-side.

### C.5 Live API conversation (audio in, audio/text out) + ephemeral tokens

Current Live models: `gemini-3.1-flash-live-preview` (recommended; `thinkingLevel`, default `minimal`) and `gemini-2.5-flash-native-audio-preview-12-2025`. Both are Preview; "The Live API is in Preview." Specs: input audio raw 16-bit PCM 16 kHz LE; output audio raw 16-bit PCM **24 kHz** LE; images JPEG ≤ 1 fps; text. Native-audio models "only support `AUDIO` response modality" — get text via `outputAudioTranscription`. Session limits without compression: audio-only 15 min, audio+video 2 min; connection lifetime ≈ 10 min (use `sessionResumption`; handles valid 2 h); `contextWindowCompression: { slidingWindow: {} }` for unlimited sessions; context window 128k for native-audio models. 3.1 Live: `sendClientContent` only for seeding history (requires `historyConfig.initialHistoryInClientContent: true`), then `sendRealtimeInput({ text })`; function calling synchronous only; a single server event may contain several parts.

Connect (verbatim, capabilities guide):

```javascript
import { GoogleGenAI, Modality } from '@google/genai';

const ai = new GoogleGenAI({});
const model = 'gemini-3.1-flash-live-preview';
const config = { responseModalities: [Modality.AUDIO] };

async function main() {

  const session = await ai.live.connect({
    model: model,
    callbacks: {
      onopen: function () {
        console.debug('Opened');
      },
      onmessage: function (message) {
        console.debug(message);
      },
      onerror: function (e) {
        console.debug('Error:', e.message);
      },
      onclose: function (e) {
        console.debug('Close:', e.reason);
      },
    },
    config: config,
  });

  console.debug("Session started");
  // Send content...

  session.close();
}

main();
```

Send audio / receive audio / transcripts (verbatim):

```javascript
// Assuming 'chunk' is a Buffer of raw PCM audio
session.sendRealtimeInput({
  audio: {
    data: chunk.toString('base64'),
    mimeType: 'audio/pcm;rate=16000'
  }
});
```

```javascript
// Inside the onmessage callback
const content = response.serverContent;
if (content?.modelTurn?.parts) {
  for (const part of content.modelTurn.parts) {
    if (part.inlineData) {
      const audioData = part.inlineData.data;
      // Process or play audioData (base64 encoded string)
    }
  }
}
```

```javascript
const config = {
  responseModalities: [Modality.AUDIO],
  outputAudioTranscription: {}
};
```

```javascript
const config = {
  responseModalities: [Modality.AUDIO],
  inputAudioTranscription: {}
};
```

```javascript
const model = 'gemini-3.1-flash-live-preview';
const config = {
  responseModalities: [Modality.AUDIO],
  thinkingConfig: {
    thinkingLevel: 'low',
  },
};
```

```javascript
import { GoogleGenAI, Modality, StartSensitivity, EndSensitivity } from '@google/genai';

const config = {
  responseModalities: [Modality.AUDIO],
  realtimeInputConfig: {
    automaticActivityDetection: {
      disabled: false, // default
      startOfSpeechSensitivity: StartSensitivity.START_SENSITIVITY_LOW,
      endOfSpeechSensitivity: EndSensitivity.END_SENSITIVITY_LOW,
      prefixPaddingMs: 20,
      silenceDurationMs: 100,
    }
  }
};
```

```javascript
const config = {
  responseModalities: [Modality.AUDIO],
  contextWindowCompression: { slidingWindow: {} }
};
```

Tool use in Live (verbatim excerpt, Live tools guide): declare `tools: [{ functionDeclarations: [...] }]` in the session config, then on `message.toolCall` reply with

```javascript
      session.sendToolResponse({ functionResponses: functionResponses });
```

where each response is `{ id: fc.id, name: fc.name, response: { result: "ok" } }`. Search grounding is supported in Live; Maps, code execution, URL context are not.

Ephemeral tokens (verbatim, ephemeral-tokens guide). Defaults: `uses: 1`, `expireTime` 30 min, `newSessionExpireTime` 1 min; max 20 h; "only compatible with Live API", `v1beta` only; the client passes the token as the `apiKey` in the SDK, or as `?access_token=` on the `...BidiGenerateContentConstrained` WebSocket endpoint, or HTTP `Authorization: Token <token>`:

```javascript
import { GoogleGenAI } from "@google/genai";

const client = new GoogleGenAI({});
const expireTime = new Date(Date.now() + 30 * 60 * 1000).toISOString();

const token = await client.authTokens.create({
    config: {
      uses: 1, // The default
      expireTime: expireTime, // Default is 30 mins
      newSessionExpireTime: new Date(Date.now() + (1 * 60 * 1000)), // Default 1 minute in the future
    },
  });
```

```javascript
import { GoogleGenAI, Modality } from '@google/genai';

// Use the token generated in the "Create an ephemeral token" section here
const ai = new GoogleGenAI({
  apiKey: token.name
});
const model = 'gemini-3.1-flash-live-preview';
const config = { responseModalities: [Modality.AUDIO] };

async function main() {

  const session = await ai.live.connect({
    model: model,
    config: config,
    callbacks: { ... },
  });

  // Send content...

  session.close();
}

main();
```

"Within the `expireTime` timeframe, you'll need `sessionResumption` to reconnect the call every 10 minutes (this can be done with the same token even if `uses: 1`)." Locking `liveConnectConstraints` lets you keep the system instruction server-side.

### C.6 Embeddings (verbatim, Embeddings guide)

```javascript
import { GoogleGenAI } from "@google/genai";

async function main() {

    const ai = new GoogleGenAI({});

    const response = await ai.models.embedContent({
        model: 'gemini-embedding-2',
        contents: 'What is the meaning of life?',
    });

    console.log(response.embeddings);
}

main();
```

```javascript
import { GoogleGenAI } from "@google/genai";

async function main() {
    const ai = new GoogleGenAI({});

    const response = await ai.models.embedContent({
        model: 'gemini-embedding-2',
        contents: 'What is the meaning of life?',
        config: { outputDimensionality: 768 },
    });

    const embeddingLength = response.embeddings[0].values.length;
    console.log(`Length of embedding: ${embeddingLength}`);
}

main();
```

Several separate embeddings in one call with embedding-2 — wrap each input in a `Content` (verbatim):

```javascript
    const response = await ai.models.embedContent({
        model: 'gemini-embedding-2',
        contents: [
            { parts: [{ text: 'task: classification | query: An image of a dog' }] },
            {
                parts: [{
                    inlineData: {
                        mimeType: 'image/png',
                        data: imgBase64,
                    },
                }],
            },
        ],
    });

    // This produces two embeddings
    for (const embedding of response.embeddings) {
        console.log(embedding.values);
    }
```

embedding-001 with `taskType` (verbatim):

```javascript
    const response = await ai.models.embedContent({
        model: 'gemini-embedding-001',
        contents: texts,
        config: { taskType: 'SEMANTIC_SIMILARITY' },
    });

    const embeddings = response.embeddings.map(e => e.values);
```

Task prefixes for embedding-2 (text-only): queries `task: search result | query: {content}` (also `question answering`, `fact checking`, `code retrieval`, `classification`, `clustering`, `sentence similarity`); documents `title: {title} | text: {content}` (`title: none` if none). Do not prefix the text portion of multimodal inputs. REST batch endpoint: `POST /v1beta/models/gemini-embedding-2:batchEmbedContents` with `{"requests":[{"model":"models/gemini-embedding-2","content":{"parts":[...]}}, ...]}`. Batch API (50% off) also supports embeddings.

### C.7 Structured output

New generateContent shape (verbatim, legacy Structured outputs guide — uses `responseFormat.text`; the API reference marks `responseSchema` and `_responseJsonSchema` deprecated while `responseMimeType` + `responseJsonSchema` are still accepted and are what Google's codegen instructions use):

```javascript
import { GoogleGenAI } from "@google/genai";
import { z } from "zod";
import { zodToJsonSchema } from "zod-to-json-schema";

const feedbackSchema = z.object({
  sentiment: z.enum(["positive", "neutral", "negative"]),
  summary: z.string(),
});

const ai = new GoogleGenAI({});
const prompt = "The new UI is incredibly intuitive and visually appealing. Great job! Add a very long summary to test streaming!";

const stream = await ai.models.generateContentStream({
  model: "gemini-3.8-flash",
  contents: prompt,
  config: {
    responseFormat: { text: { mimeType: "application/json", schema: zodToJsonSchema(feedbackSchema) } },
  },
});

for await (const chunk of stream) {
  console.log(chunk.candidates[0].content.parts[0].text)
}
```

Older-but-still-valid shape (verbatim, js-genai codegen instructions):

```javascript
  const response = await ai.models.generateContent({
    model: "gemini-3-flash-preview",
    contents: "List a few popular cookie recipes, and include the amounts of ingredients.",
    config: {
      responseMimeType: "application/json",
      responseJsonSchema: {
          type: Type.ARRAY,
          items: {
            type: Type.OBJECT,
            properties: {
              recipeName: {
                type: Type.STRING,
                description: 'The name of the recipe.',
              },
              ingredients: {
                type: Type.ARRAY,
                items: {
                  type: Type.STRING,
                },
                description: 'The ingredients for the recipe.',
              },
            },
            propertyOrdering: ["recipeName", "ingredients"],
          },
        },
    },
  });
```

Interactions shape (verbatim):

```javascript
const interaction = await client.interactions.create({
  model: "gemini-3.8-flash",
  input: prompt,
  response_format: {
    type: 'text',
    mime_type: 'application/json',
    schema: recipeJsonSchema
  },
});

const recipe = recipeSchema.parse(JSON.parse(interaction.output_text));
```

Supported JSON Schema subset: `type` (string, number, integer, boolean, object, array, null via type arrays), `title`, `description`, `properties`, `required`, `additionalProperties`, `enum` (strings and numbers), `format` (date-time/date/time), `minimum`/`maximum`, `items`, `prefixItems`, `minItems`/`maxItems`, `$ref`/`$defs`/`$id`/`$anchor`, `anyOf` (`oneOf` treated as `anyOf`), non-standard `propertyOrdering`. Enum-only output: `responseMimeType: "text/x.enum"` still exists in the GenerationConfig reference (or just use a string `enum` in a JSON schema). Structured outputs can be combined with built-in tools and function calling on Gemini 3 models. Streaming chunks are valid partial JSON to concatenate.

### C.8 Function calling

generateContent (verbatim, legacy Function calling guide):

```javascript
import { Type } from '@google/genai';

// Define a function that the model can call to control smart lights
const setLightValuesFunctionDeclaration = {
  name: 'set_light_values',
  description: 'Sets the brightness and color temperature of a light.',
  parameters: {
    type: Type.OBJECT,
    properties: {
      brightness: {
        type: Type.NUMBER,
        description: 'Light level from 0 to 100. Zero is off and 100 is full brightness',
      },
      color_temp: {
        type: Type.STRING,
        enum: ['daylight', 'cool', 'warm'],
        description: 'Color temperature of the light fixture, which can be `daylight`, `cool` or `warm`.',
      },
    },
    required: ['brightness', 'color_temp'],
  },
};
```

```javascript
import { GoogleGenAI } from '@google/genai';

// Generation config with function declaration
const config = {
  tools: [{
    functionDeclarations: [setLightValuesFunctionDeclaration]
  }]
};

// Configure the client
const ai = new GoogleGenAI({});

// Define user prompt
const contents = [
  {
    role: 'user',
    parts: [{ text: 'Turn the lights down to a romantic level' }]
  }
];

// Send request with function declarations
const response = await ai.models.generateContent({
  model: 'gemini-3.8-flash',
  contents: contents,
  config: config
});

console.log(response.functionCalls[0]);
```

```javascript
// Create a function response part
const function_response_part = {
  name: tool_call.name,
  response: { result },
  id: tool_call.id
}

// Append function call and result of the function execution to contents
contents.push(response.candidates[0].content);
contents.push({ role: 'user', parts: [{ functionResponse: function_response_part }] });

// Get the final response from the model
const final_response = await ai.models.generateContent({
  model: 'gemini-3.8-flash',
  contents: contents,
  config: config
});

console.log(final_response.text);
```

Rules (verbatim gist): push `response.candidates[0].content` back unchanged (it carries `thoughtSignature`); "Always include the exact `id` from the `function_call` in your `function_response`"; don't merge parts with/without signatures. Mismatched id/name/count on Gemini 3.x ⇒ empty response with `finishReason: STOP`. Function calling mode: `config.toolConfig.functionCallingConfig.mode: FunctionCallingConfigMode.ANY` + `allowedFunctionNames` (README sample). `parametersJsonSchema` is also accepted on `FunctionDeclaration` (README).

Interactions (verbatim): tools are `{ type: 'function', name, description, parameters }`; responses are `{ type: 'function_result', name, call_id: fcStep.id, result: [{ type: 'text', text: JSON.stringify(result) }] }` with `previous_interaction_id`; `generation_config.tool_choice` = `auto | any | none | validated` or `{ allowed_tools: { mode: 'any', tools: [...] } }`:

```javascript
const finalInteraction = await client.interactions.create({
  model: 'gemini-3.8-flash',
  input: [{
    type: 'function_result',
    name: fcStep.name,
    call_id: fcStep.id,
    result: [{ type: 'text', text: JSON.stringify(result) }]
  }],
  tools: [setLightValuesTool],
  previous_interaction_id: interaction.id,
});
```

### C.9 Files API (verbatim, Files guide)

```javascript
import { GoogleGenAI } from "@google/genai";

const client = new GoogleGenAI({});

async function main() {
  const myfile = await client.files.upload({
    file: "path/to/sample.mp3",
    config: { mime_type: "audio/mpeg" },
  });

  const interaction = await client.interactions.create({
    model: "gemini-3.8-flash",
    input: [
      { type: "text", text: "Describe this audio clip" },
      { type: "audio", uri: myfile.uri, mime_type: myfile.mimeType }
    ]
  });
  console.log(interaction.output_text);
}

await main();
```

```javascript
import {
  GoogleGenAI,
  createUserContent,
  createPartFromUri,
} from "@google/genai";

const ai = new GoogleGenAI({});

async function main() {
  const myfile = await ai.files.upload({
    file: "path/to/sample.jpg",
    config: { mimeType: "image/jpeg" },
  });

  const response = await ai.models.generateContent({
    model: "gemini-3.8-flash",
    contents: createUserContent([
      createPartFromUri(myfile.uri, myfile.mimeType),
      "Caption this image.",
    ]),
  });
  console.log(response.text);
}

await main();
```

Also `ai.files.get({ name })`, `ai.files.list({ config: { pageSize } })`, `ai.files.delete({ name })`. Facts: "store up to 20 GB of files per project, with a per-file maximum size of 2 GB. Files are stored for 48 hours"; user uploads cannot be downloaded; free in all regions. Use the Files API "when the total request size ... is larger than 100 MB. For PDF files, the limit is 50 MB" (Files guide) — older guides still say 20 MB for inline. Uploaded video/large files may need polling until `state !== 'PROCESSING'`. In the browser, `file` can be a `Blob`/`File` (**UNVERIFIED** from fetched pages — the docs only show path strings; the SDK web build exists so a Blob path is expected).

### C.10 Context caching

Implicit caching is on by default for 2.5+ models; minimum prompt size **4,096 tokens** for 3.8/3.7/3.6/3.5 Flash and 3.1 Pro (2,048 for 2.5 Flash/Pro); put static content first and send similar prefixes close in time; hits show in `usageMetadata.cachedContentTokenCount` (Interactions: `usage.total_cached_tokens`). Explicit caching (`ai.caches.create({ model, config: { contents, systemInstruction, ttl } })` then `config: { cachedContent: cache.name }`) is generateContent-only; default TTL 1 h; storage billed per 1M tokens/hour. Interactions API "only supports implicit caching".

## (D) REST shapes for Rust / reqwest

Auth: header `x-goog-api-key: $GEMINI_API_KEY` (a `?key=` query parameter also appears in reference examples; the header is preferred). All JSON; field names are camelCase in generateContent/Live, snake_case in Interactions.

**generateContent (unary)**

```
POST https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:generateContent
x-goog-api-key: $GEMINI_API_KEY
Content-Type: application/json

{
  "system_instruction": { "parts": [ { "text": "You are a cat. Your name is Neko." } ] },
  "contents": [
    { "role": "user",  "parts": [ { "text": "Hello" } ] },
    { "role": "model", "parts": [ { "text": "Great to meet you. What would you like to know?" } ] },
    { "role": "user",  "parts": [
        { "inline_data": { "mime_type": "image/png", "data": "<base64>" } },
        { "text": "What is in this screenshot?" }
    ] }
  ],
  "generationConfig": {
    "thinkingConfig": { "thinkingLevel": "low" },
    "maxOutputTokens": 1000,
    "responseFormat": { "text": { "mimeType": "application/json", "schema": { "type": "object", "properties": { "answer": { "type": "string" } }, "required": ["answer"] } } }
  }
}
```

Request body fields: `contents[]`, `tools[]`, `toolConfig`, `safetySettings[]`, `systemInstruction`, `generationConfig`, `cachedContent`, `serviceTier` (`flex|standard|priority`), `store`. `Part` = `{ thought?, thoughtSignature?, mediaResolution?, text | inlineData{mimeType,data} | functionCall | functionResponse | fileData{fileUri,mimeType} | ... }`. `GenerationConfig` includes `stopSequences`, `responseMimeType`, `responseSchema` (deprecated), `responseJsonSchema`, `responseFormat`, `responseModalities`, `candidateCount` (unsupported on 3.x), `maxOutputTokens`, `temperature`/`topP`/`topK` (deprecated for 3.x), `seed`, `presencePenalty`, `frequencyPenalty`, `thinkingConfig{includeThoughts, thinkingBudget, thinkingLevel: MINIMAL|LOW|MEDIUM|HIGH}`, `mediaResolution` (enum `MEDIA_RESOLUTION_LOW|MEDIUM|HIGH`), `audioTranscriptionConfig{languageCodes[], customVocabulary[], wordTimestamp, diarization, mode: VERBATIM|SMART}`, `speechConfig`, `imageConfig`.

Response (verbatim shape):

```json
{
  "candidates": [
    {
      "content": { "parts": [ { "text": "..." } ], "role": "model" },
      "finishReason": "STOP",
      "index": 0
    }
  ],
  "usageMetadata": { "promptTokenCount": 4, "candidatesTokenCount": 12, "thoughtsTokenCount": 0, "totalTokenCount": 16 },
  "modelVersion": "gemini-3.8-flash",
  "responseId": "..."
}
```

Model parts may include `"thought": true` (summary) and `"thoughtSignature": "<base64>"` — echo them back verbatim in `contents` on the next turn. Function calls arrive as `{ "functionCall": { "id", "name", "args" } }`; reply with `{ "functionResponse": { "id", "name", "response": {...} } }` in a `user` turn.

**Streaming (SSE)**

```
POST https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:streamGenerateContent?alt=sse
x-goog-api-key: $GEMINI_API_KEY
Content-Type: application/json
(same body)
```

Each SSE `data:` line is a full `GenerateContentResponse` JSON chunk sharing one `responseId`; concatenate `candidates[0].content.parts[*].text`. Without `?alt=sse` the endpoint returns a JSON array streamed in pieces. Use a `--no-buffer`-style reader (reqwest `bytes_stream()` + an SSE line parser, or the `eventsource-stream` crate).

**Transcription via generateContent**

```
POST https://generativelanguage.googleapis.com/v1beta/models/gemini-3.5-transcribe:generateContent
{
  "contents": [ { "parts": [ { "fileData": { "fileUri": "YOUR_FILE_URI", "mimeType": "audio/mp3" } } ] } ],
  "generationConfig": { "audioTranscriptionConfig": { "languageCodes": [], "diarization": true, "wordTimestamp": true } }
}
```

(`inline_data` with base64 audio also works for small files.) Response parts carry `audioTranscription: { speakerLabel, words: [{ word, startOffset: "0.100s", endOffset: "0.450s" }] }`; plain transcript text is in text parts.

**Interactions API**

```
POST https://generativelanguage.googleapis.com/v1beta/interactions          (unary)
POST https://generativelanguage.googleapis.com/v1beta/interactions?alt=sse  (with "stream": true)
{
  "model": "gemini-3.8-flash",
  "system_instruction": "...",
  "input": [ { "type": "text", "text": "..." }, { "type": "image", "data": "<base64>", "mime_type": "image/png", "resolution": "high" } ],
  "generation_config": { "thinking_level": "low", "thinking_summaries": "none" },
  "response_format": { "type": "text", "mime_type": "application/json", "schema": {...} },
  "previous_interaction_id": "v1_...", "store": true, "stream": false, "service_tier": "standard"
}
```

Response: `{ "id", "status": "completed", "steps": [ { "type": "thought", "signature": "...", "summary": [...] }, { "type": "model_output", "content": [ { "type": "text", "text": "..." } ] } ], "usage": {...} }`. `GET /v1beta/interactions/{id}`, `DELETE`, and `POST .../{id}:cancel` (**UNVERIFIED exact cancel path**) exist. Content types: `text`, `image{data|uri, mime_type, resolution}`, `audio{data|uri, mime_type, sample_rate?, channels?}`, `video`, `document{application/pdf|text/csv}`. Errors: `{ "error": { "code": "rate_limit_exceeded", "message": "..." } }` with snake_case codes (`invalid_request`, `authentication`, `permission_denied`, `model_not_found`, `rate_limit_exceeded`, `quota_exceeded`, `too_many_requests`, `api_error`, `service_unavailable`, `deadline_exceeded`; generation codes `malformed_function_call`, `missing_thought_signature`, ...). Note the migration guide's REST sample shows `/v1beta2/interactions` once — probably a typo; every other page says `/v1beta`.

**Embeddings**

```
POST https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-2:embedContent
{ "content": { "parts": [ { "text": "What is the meaning of life?" } ] }, "output_dimensionality": 768 }
→ { "embedding": { "values": [ ... ] } }        (batchEmbedContents → { "embeddings": [ { "values": [...] }, ... ] })

POST https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-001:embedContent
{ "taskType": "SEMANTIC_SIMILARITY", "content": { "parts": [ { "text": "..." } ] } }
```

(Response key names `embedding`/`embeddings` are the long-standing REST shape; the guide only prints SDK output — **UNVERIFIED** for this snapshot.)

**Files API (resumable upload, verbatim protocol)**

```
POST https://generativelanguage.googleapis.com/upload/v1beta/files
  x-goog-api-key, X-Goog-Upload-Protocol: resumable, X-Goog-Upload-Command: start,
  X-Goog-Upload-Header-Content-Length: <bytes>, X-Goog-Upload-Header-Content-Type: <mime>
  body: {"file": {"display_name": "AUDIO"}}
→ response header x-goog-upload-url
POST <upload_url>  Content-Length, X-Goog-Upload-Offset: 0, X-Goog-Upload-Command: upload, finalize, --data-binary @file
→ {"file": {"name": "files/...", "uri": "https://generativelanguage.googleapis.com/v1beta/files/...", "mimeType": ..., "state": ...}}
GET/DELETE https://generativelanguage.googleapis.com/v1beta/files/{id}
```

**Ephemeral tokens**

```
POST https://generativelanguage.googleapis.com/v1beta/auth_tokens
{ "uses": 1, "expireTime": "YYYY-MM-DDTHH:MM:SSZ", "newSessionExpireTime": "...",
  "liveConnectConstraints": { "model": "models/gemini-3.5-transcribe-live", "config": { "responseModalities": ["TEXT"], "inputAudioTranscription": { "languageCodes": [] } } } }
→ { "name": "<token>" , ... }
```

**Live API WebSocket** (verified on the Live reference, the WebSocket get-started page, and the live-transcribe guide):

```
wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent?key=YOUR_API_KEY
wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContentConstrained?access_token={ephemeral-token}
```

Client messages — exactly one of `setup`, `clientContent`, `realtimeInput`, `toolResponse` per JSON message. First message must be `setup`; wait for `setupComplete` before sending more:

```json
{ "setup": {
    "model": "models/gemini-3.5-transcribe-live",
    "generationConfig": { "responseModalities": ["TEXT"] },
    "systemInstruction": { "parts": [ { "text": "..." } ] },
    "tools": [],
    "realtimeInputConfig": { "automaticActivityDetection": { "disabled": false, "startOfSpeechSensitivity": "START_SENSITIVITY_HIGH", "endOfSpeechSensitivity": "END_SENSITIVITY_HIGH", "prefixPaddingMs": 20, "silenceDurationMs": 800 }, "activityHandling": "START_OF_ACTIVITY_INTERRUPTS", "turnCoverage": "TURN_INCLUDES_ONLY_ACTIVITY" },
    "inputAudioTranscription": { "languageCodes": [], "customVocabulary": [], "mode": "VERBATIM" },
    "outputAudioTranscription": {},
    "sessionResumption": { "handle": "<previous handle or omit>" },
    "contextWindowCompression": { "slidingWindow": { "targetTokens": 8000 }, "triggerTokens": 25000 },
    "historyConfig": { "initialHistoryInClientContent": true }
} }
{ "realtimeInput": { "audio": { "data": "<base64 PCM16 LE 16k mono>", "mimeType": "audio/pcm;rate=16000" } } }
{ "realtimeInput": { "audioStreamEnd": true } }
{ "realtimeInput": { "activityStart": {} } }   /  { "realtimeInput": { "activityEnd": {} } }
{ "realtimeInput": { "text": "Hello" } }
{ "realtimeInput": { "video": { "data": "<base64 jpeg>", "mimeType": "image/jpeg" } } }
{ "clientContent": { "turns": [ { "role": "user", "parts": [ { "text": "..." } ] } ], "turnComplete": true } }
{ "toolResponse": { "functionResponses": [ { "id": "...", "name": "...", "response": { "result": "ok" } } ] } }
```

Server messages (`BidiGenerateContentServerMessage`; exactly one of the union fields plus optional `usageMetadata`): `setupComplete`; `serverContent { modelTurn{parts[inlineData audio/pcm 24 kHz | text]}, inputTranscription{text, languageCode}, interimInputTranscription{text}, outputTranscription{text}, interrupted, generationComplete, turnComplete, waitingForInput, groundingMetadata }`; `toolCall { functionCalls[] }`; `toolCallCancellation { ids[] }`; `goAway { timeLeft }`; `sessionResumptionUpdate { newHandle, resumable }`. `setup.generationConfig` does not support `responseMimeType`, `responseSchema`, `responseJsonSchema`, `stopSequence`, logprobs. `mediaChunks[]` is deprecated in favour of `audio`/`video`/`text`.

## (E) OpenAI-compatibility endpoint

Base URL `https://generativelanguage.googleapis.com/v1beta/openai/`, `Authorization: Bearer $GEMINI_API_KEY`, "still in beta". Works: `chat/completions` (system role, `stream: true`, `tools`/`tool_choice`, `image_url` with `data:image/jpeg;base64,...`, `input_audio: { data, format: "wav" }` for audio understanding, `response_format` structured outputs incl. `zodResponseFormat`, `reasoning_effort` `minimal|low|medium|high` mapped to `thinking_level` (`"none"` only for 2.5 models; "Reasoning cannot be turned off for ... 3 models"), `extra_body.google.thinking_config{thinking_level, include_thoughts}` and `cached_content`, `service_tier` `flex|priority`, thought signatures supported in chat completions for Gemini 3); `embeddings` (docs use `gemini-embedding-2-preview`, which is shut down — use `gemini-embedding-2` or `gemini-embedding-001`; `dimensions` param not documented — **UNVERIFIED**); `images/generations`; `videos` (Sora-compatible); `batches` (create/status/results only — file upload/download must use the genai SDK); `models` list/retrieve. **Not offered**: WebSocket/Realtime, `/audio/transcriptions` or `/audio/speech`, Files, Live — none appear on the page. Unknown parameters "will be silently ignored by the compatibility layer" (images section). Google recommends calling the Gemini API directly if you are not already on the OpenAI libraries. Verdict: an existing OpenAI-SDK chat/vision/embeddings path can be pointed at Gemini in one line for a quick spike, but transcription (`gemini-3.5-transcribe` config, Live transcribe) needs the native API.

## (F) Rust crate options

| Crate | Version (date) | Maintenance | Streaming | Audio/image input | Live API (WebSocket) | Notes |
|---|---|---|---|---|---|---|
| `gemini-rust` (flachesis) | **2.1.0** (2026-09-05); 2.0.0 2026-07-10 | active; MIT; 77 stars; 6 open issues; ~49k downloads | Yes — SSE via `eventsource-stream` for both `generate_content_stream` and Interactions step events | Yes — `interaction_multimodal.rs` (image, audio, video input), file handles, embeddings (`gemini-embedding-2`/`001`, batch) | **No** — no WebSocket dependency (reqwest + rustls only); README never mentions Live | Full Interactions API (server-side state, thinking levels, media resolution, structured output via `.with_json_schema()`, function calling, files, caching, batch), legacy `generateContent` "still available but deprecated" in its README. Model enum lists 3.7 Flash as default; 3.8 needs `Model::Custom("gemini-3.8-flash")` until updated. `transcription_config` support **UNVERIFIED** |
| `genai` (jeremychone/rust-genai) | **0.6.5** stable; 0.7.0-beta.24 (2026-09-08) | very active; Apache-2.0; 877 stars; ~358k downloads | Yes — `exec_chat_stream` | Yes — `ContentPart::Binary` (image/PDF/audio, base64 or URL); embeddings with Gemini `embedding_type` | **No** | Multi-provider (native Gemini protocol = generateContent, not Interactions); thought-signature parts round-trip; JSON-schema normalisation for Gemini; no transcription config; 0.7 has fallible `Client::new()?` |
| `gemini-rs` (Shuflduf) | 2.0.1 (2026-07-21) | low volume | ? | ? | No | Not evaluated further |
| `google-generative-ai-rs` | 0.3.4 (2024-12-23) | stale | | | | avoid |
| `gemini-client-api` | 7.5.5 (2026-08-26) | small | | | | not evaluated |
| `rig-core` / `rig-gemini-grpc` 0.42 | 2026-08-17 | active | | | | agent framework; heavier than needed |
| `googleapis-tonic-google-ai-generativelanguage-v1beta` 0.42 | 2026-06-20 | generated | gRPC | | gRPC bidi possible but undocumented for API keys | only if you want gRPC |

Recommendation: for this app the Rust side needs (a) `generateContent`/`streamGenerateContent` (chat, vision, transcription of files) — either `gemini-rust` 2.1 or plain `reqwest` + `serde_json` + an SSE parser, and (b) Live WebSocket for real-time transcription — no crate covers it; use `tokio-tungstenite` (with `rustls`) against the URL in (D), sending the `setup` message first and JSON frames thereafter. Given the Interactions/generateContent split and the frequent schema churn, hand-rolled `serde` structs over the small REST subset you use is the lowest-risk path; keep model IDs and config as data, not enums.

## (G) Gotchas & deprecations

1. **Old SDKs**: `@google/generative-ai` (and Python `google-generativeai`) are "deprecated as of November 30th, 2025" and lack Live/3.x features. Use `@google/genai` 2.x; pin `<3.0.0` (v3 needs Node 22 and removes AFC-from-generateContent, `LiveConnectConfig.generation_config`, `GenerationConfigThinkingConfig`).
2. **API keys**: new AI Studio keys are auth keys; unrestricted standard keys are already rejected; all standard keys rejected in September 2026 — regenerate the key. Do not ship the key in the WebView bundle; keep it in the Rust/Tauri side (OS keychain) and, for browser-side Live sessions, mint ephemeral tokens (Live only; `v1beta` only; 30 min default life; `uses: 1`).
3. **Two API surfaces**: Interactions (`/v1beta/interactions`, `ai.interactions.create`, snake_case, `steps`, `output_text`, server-side state, 1-day free / 55-day paid storage unless `store:false`) vs legacy generateContent (`ai.models.generateContent`, camelCase, `candidates`, client-managed history, explicit caching, Batch, safety settings). Pick one per feature; do not mix shapes. The docs toggle silently between them.
4. **Thinking**: `thinking_level` replaces `thinking_budget` on 3.x; sending both ⇒ 400; `minimal` errors on 3.7/3.8 (fine on 3.6/3.5/3.5-lite); default is `medium` on all 3.5+ Flash (was `high` on 3-flash-preview); thinking tokens are billed as output and "3.8 Flash can use more tokens on longer running and complex tasks, by design" — use `low` for chat UI latency. Thinking cannot be fully disabled on Gemini 3 (`minimal` ≠ off).
5. **Sampling params**: `temperature`, `top_p`, `top_k` deprecated (2026-07-21) and "strongly recommend not changing"; leave at default 1.0 (lower values can loop/degrade); `candidate_count` unsupported on 3.x; the troubleshooting page's "Temperature 0.0–1.0" table is stale (API allows 0–2).
6. **Thought signatures**: SDK handles them if you resend whole model turns; if you hand-build history (Rust), keep `thoughtSignature` on every part, keep `functionCall.id` ↔ `functionResponse.id`, never merge/split signed parts. Gemini 3.5+ uses signatures from all previous turns ("thought preservation" — raises input tokens). Interactions stateful mode needs nothing; stateless needs every `thought` step resent.
7. **Function calling on 3.x**: id + name + count must match; multimodal results go inside the function response; append extra instructions to the function-result text after `\n\n`; `Malformed_Function_Call` when forcing structured text right before a tool call — wrap notes in a dedicated function.
8. **Inline size limits**: image/audio guides say 20 MB total request; Files/file-input pages say 100 MB (50 MB PDF) since 2026-01-08. Screenshots are far below either; audio files > ~20 MB should go through the Files API (48 h, 2 GB/file, 20 GB/project).
9. **Media resolution**: Gemini 3 defaults images to 1,120 tokens (= `high`); per-part `resolution` (`low|medium|high|ultra_high`) only on Gemini 3; the generateContent API-reference enum text still describes 2.x token counts (64/256) — trust the media-resolution guide table. PDFs bill at image rates.
10. **Transcribe feature exclusivity**: custom vocabulary ⊥ diarization/timestamps (request rejected); SMART ⊥ diarization/timestamps; 30-min cap when diarization/timestamps on; Live transcribe has no diarization/word timestamps and a hard 10-minute session (plan reconnect + client-side stitching).
11. **Live audio formats**: input PCM16 LE 16 kHz mono (`audio/pcm;rate=16000`; other rates accepted if declared), output PCM16 24 kHz; resample mic (44.1/48 kHz) client-side; send 20–100 ms chunks (transcribe guide says 100 ms); on `interrupted: true` flush playback; audio-only 15 min / connection ~10 min without compression + resumption; native-audio models only output AUDIO (use `outputAudioTranscription` for text); 3.1 Live drops proactive audio / affective dialog / async tools and may pack several parts per event.
12. **Alias drift**: `gemini-flash-latest` hot-swaps (last recorded → `gemini-3.5-flash` on 2026-05-19; current target UNVERIFIED). Pin `gemini-3.8-flash`. No `-preview` suffix exists for 3.8.
13. **Pricing cliff**: 3.8/3.7/3.6 Flash double in price on 2027-01-01 ($0.75/$3.75 → $1.50/$7.50). 3.5 Flash is already $1.50/$9.00 — there is no reason to prefer it. Grounding with Google Search: 5,000 free requests/month shared across Gemini 3.x, then $14/1k (not on free tier).
14. **Embeddings**: `gemini-embedding-2` returns ONE vector for multiple parts unless each is wrapped in its own `Content`; no `taskType` (use prompt prefixes; be consistent between indexing and querying); spaces incompatible with `gemini-embedding-001` (re-embed on migration); `-preview` ID is shut down but still appears in some docs; 001 needs manual normalization below 3072 dims.
15. **Rate limits**: no official per-model numbers; third-party measurements put Free at 5 RPM/20 RPD for the Flash line — link billing (Tier 1) before testing anything real. Spend caps ($10/10 min at Tier 1) also return 429. Retry 408/429/5xx with exponential backoff + jitter; never retry 400/403.
16. **Regions**: the Gemini API/AI Studio availability list includes the US, UK, all EU members and most of the world; use the published list (section H) if you gate by locale — Vertex is the fallback for unsupported regions. Google AI Studio itself requires 18+.
17. **Docs inconsistencies to expect**: JS Interactions samples mix snake_case and camelCase; legacy pages still show `gemini-2.0-flash` in REST examples; OpenAI page uses a shut-down embedding ID; the js-genai codegen instructions recommend `gemini-3-flash-preview`; `whats-new-gemini-3.5` calls itself the migration checklist while `latest-model` covers 3.8. Always verify field names against the SDK's TypeScript types.
18. **Structured output field churn**: `responseSchema` deprecated → `responseJsonSchema` (still fine) → new `responseFormat.text.{mimeType, schema}` (generateContent) / `response_format{type:"text", mime_type, schema}` (Interactions; `response_mime_type` removed there in May 2026).
19. **Image segmentation** is not supported on Gemini 3.x; bounding boxes are.
20. **Bun**: untested/undocumented for `@google/genai`; the SDK's Node Live path uses the `ws` package; the web build uses native WebSocket. In a Tauri WebView the `./web` build applies.

## (H) Sources (all fetched 2026-09-08 via direct HTTPS GET; Exa used only for the rate-limit search)

Official Google (ai.google.dev):
- https://ai.google.dev/gemini-api/docs/changelog — release notes (3.8 Flash GA 2026-09-02; 3.5 Transcribe GA 2026-08-26; 3.7 Flash GA 2026-08-13; 3.6 Flash & 3.5 Flash-Lite GA + sampling params deprecated 2026-07-21; embedding-2 GA 2026-04-22; 2.0 shutdown 2026-06-01; alias changes)
- https://ai.google.dev/gemini-api/docs/models — model catalog, endpoints, version patterns
- https://ai.google.dev/gemini-api/docs/models/gemini-3.8-flash, …/gemini-3.7-flash, …/gemini-3.6-flash, …/gemini-3.5-flash, …/gemini-3.5-flash-lite, …/gemini-3.1-flash-lite, …/gemini-3-flash-preview, …/gemini-2.5-flash, …/gemini-3.5-transcribe, …/gemini-3.1-flash-live-preview, …/gemini-2.5-flash-native-audio-preview-12-2025, …/gemini-embedding-2, …/gemini-embedding-001 — spec tables
- https://ai.google.dev/gemini-api/docs/latest-model — "What's new in Gemini 3.8 Flash" (intro pricing, thinking levels, migration checklist, sampling-parameter deprecation)
- https://ai.google.dev/gemini-api/docs/whats-new-gemini-3.5 — Gemini 3.5 migration checklist, thought preservation, parameter guidance
- https://ai.google.dev/gemini-api/docs/pricing — all prices and free-tier columns
- https://ai.google.dev/gemini-api/docs/rate-limits — tiers, spend caps, batch limits (no per-model table)
- https://ai.google.dev/gemini-api/docs/deprecations — release/shutdown dates
- https://ai.google.dev/gemini-api/docs/available-regions — country list
- https://ai.google.dev/gemini-api/docs/api-key — auth keys, September 2026 standard-key rejection, client-side guidance
- https://ai.google.dev/gemini-api/docs/libraries — SDK list, legacy deprecation 2025-11-30
- https://ai.google.dev/gemini-api/docs/interactions-overview — Interactions API GA, retention, limitations, SDK ≥ 2.3.0
- https://ai.google.dev/api/interactions-api — request body, content types, GenerationConfig (thinking_level, transcription_config), ResponseFormat
- https://ai.google.dev/gemini-api/docs/interactions-breaking-changes-may-2026 — outputs→steps, response_format
- https://ai.google.dev/gemini-api/docs/migrate-to-interactions — generateContent ↔ Interactions mapping
- https://ai.google.dev/gemini-api/docs/streaming — SSE event types
- https://ai.google.dev/gemini-api/docs/text-generation and https://ai.google.dev/gemini-api/docs/generate-content/text-generation — Interactions and legacy text/streaming/chat samples, REST shapes
- https://ai.google.dev/gemini-api/docs/thinking and https://ai.google.dev/gemini-api/docs/generate-content/thinking — thinking levels table, signatures, budgets
- https://ai.google.dev/gemini-api/docs/gemini-3 — Gemini 3 developer guide (deprecated page; temperature guidance, thought signatures, 3-family table)
- https://ai.google.dev/gemini-api/docs/thought-signatures — redirect notice to thinking#signatures
- https://ai.google.dev/gemini-api/docs/image-understanding and https://ai.google.dev/gemini-api/docs/generate-content/image-understanding — inline images, limits, tokens (vision page is identical)
- https://ai.google.dev/gemini-api/docs/media-resolution — resolution levels and token table
- https://ai.google.dev/gemini-api/docs/audio and https://ai.google.dev/gemini-api/docs/generate-content/audio — audio understanding, formats, 32 tok/s
- https://ai.google.dev/gemini-api/docs/transcribe and https://ai.google.dev/gemini-api/docs/generate-content/transcribe — Gemini 3.5 Transcribe (Interactions and generateContent shapes)
- https://ai.google.dev/gemini-api/docs/live-api/live-transcribe — Live transcription guide (SDK, WebSocket, VAD, ephemeral tokens, limits)
- https://ai.google.dev/gemini-api/docs/live-api (overview), …/live-api/capabilities, …/live-api/session-management, …/live-api/tools, …/live-api/ephemeral-tokens, …/live-api/get-started-sdk, …/live-api/get-started-websocket, …/live-api/best-practices — Live API
- https://ai.google.dev/api/live — Live API WebSocket reference (URL, message types, AudioTranscriptionConfig, AuthToken)
- https://ai.google.dev/api/generate-content — generateContent/streamGenerateContent reference (GenerationConfig, Part, Blob, FunctionResponse, ThinkingConfig, ResponseFormatConfig, AudioTranscriptionConfig)
- https://ai.google.dev/gemini-api/docs/embeddings — embeddings guide
- https://ai.google.dev/gemini-api/docs/structured-output and https://ai.google.dev/gemini-api/docs/generate-content/structured-output
- https://ai.google.dev/gemini-api/docs/function-calling and https://ai.google.dev/gemini-api/docs/generate-content/function-calling
- https://ai.google.dev/gemini-api/docs/files and https://ai.google.dev/gemini-api/docs/file-input-methods
- https://ai.google.dev/gemini-api/docs/caching and https://ai.google.dev/gemini-api/docs/generate-content/caching
- https://ai.google.dev/gemini-api/docs/openai — OpenAI compatibility
- https://ai.google.dev/gemini-api/docs/api-errors and https://ai.google.dev/gemini-api/docs/troubleshooting — error codes, retry guidance
- https://ai.google.dev/gemini-api/docs/tokens — (fetched, not quoted)

SDK / GitHub / registries:
- https://github.com/googleapis/js-genai — README.md, CHANGELOG.md, package.json (v2.21.0), codegen_instructions.md; https://api.github.com/repos/googleapis/js-genai/releases (v2.21.0 2026-09-02, v2.20.0 2026-08-31, v2.19.0 2026-08-25, v2.18.0 2026-08-19, v2.17.1 2026-08-13)
- https://googleapis.github.io/js-genai/release_docs/interfaces/types.HttpOptions.html, …/types.HttpRetryOptions.html, …/types.GenerateContentConfig.html, …/classes/errors.ApiError.html, …/index.html — typedoc (v2.21.0)
- https://crates.io/api/v1/crates?q=gemini (+ per-crate: genai, gemini-rust, gemini_rs, google-generative-ai-rs, gemini-client-api); https://github.com/flachesis/gemini-rust (README, Cargo.toml, tree, repo metadata); https://github.com/jeremychone/rust-genai (README, Cargo.toml, docs/for-llm/api-reference-for-llm.md, repo metadata)
- registry.npmjs.org was blocked by network policy in this sandbox; the npm version was taken from the GitHub `package.json`/releases instead.

Third-party (used only for the UNVERIFIED free-tier numbers):
- https://dev.to/romeroyang/geminis-free-tier-measured-20-requests-a-day-and-google-no-longer-publishes-the-number-4gf2 (2026-09-02)
- https://www.scriptbyai.com/gemini-api-free-tier-limits/ (2026-09-07)
- https://discuss.ai.google.dev/t/gemini-3-8-flash-free-tier-20-rpd-is-too-limited-for-practical-evaluation/180609 (2026-09-03)
- https://apidog.com/blog/how-to-use-gemini-3-8-flash-for-free/ and https://ofox.ai/blog/gemini-3-8-flash-api-pricing-2026/ (2026-09-03)

Raw fetched HTML/text for every page above is kept under /agent/workspace/research/raw/ (converter: /agent/workspace/research/raw/h2t.py).
