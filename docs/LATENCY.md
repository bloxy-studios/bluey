# Latency — how the ⌘↵ fast path is measured

Companion to ADR 0010. This document is the method and the numbers; the ADR is the decision.
The trace and the bench described here are built (PR 4a): every request carries a `LatencyTrace`
(`ai_requests.trace`, `ai.trace` in developer mode, p50 / p95 per stage in Settings → Advanced →
*Fast path*), and `bun run bench:fastpath` prints the percentile table. The fast-path *changes*
(ADR 0010 §3–9) follow in PR 4b / 5 and paste their before / after tables here.

## What "fast" means here

The headline number is **⌘↵ keydown → first visible token**, and it is split into what Bluey
controls (everything before the request leaves the Mac) and what it does not (the provider's
time to first token). The local half has a hard budget; the network half is reported.

| Stage | Owner of the timestamp | p50 target | p95 target |
|---|---|---|---|
| `t_shortcut` → `t_capture_done` — SCK capture + downscale + JPEG | Rust (`CaptureManager`) | ≤ 90 ms | ≤ 150 ms |
| accessibility snapshot, raced with an 80 ms soft deadline | Rust | ≤ 80 ms or omitted | — |
| OCR on the critical path | — | 0 | 0 |
| `t_retrieval_done` — keyword FTS5 + cached query embedding, in parallel with capture | Rust / TS | ≤ 15 ms | — |
| `t_snapshot_ready` → `t_prompt_built` — fuse, budget, prompt | TS (`engine.ts`) | ≤ 15 ms | ≤ 30 ms |
| IPC + base64 + serialization, image ≤ 250 KB | Rust / TS | ≤ 20 ms | ≤ 40 ms |
| **`t_shortcut` → `t_request_sent` — local total** | | **≤ 150 ms** | **≤ 300 ms** |
| `t_request_sent` → `t_response_headers` — connect (pre-warmed) + upload | Rust | ≤ 60 ms | ≤ 120 ms |
| `t_response_headers` → `t_first_token` — provider TTFT | provider | 300–700 ms, measured | ≤ 1200 ms |
| `t_first_token` → `t_first_paint` | TS (HUD) | ≤ 16 ms | — |
| **`t_shortcut` → `t_first_paint`** | | **≤ 800 ms** | **≤ 1500 ms** |

Conditions: warm connection (a warm-up request within the last 45 s), `gemini-3.5-flash-lite`,
`thinkingLevel: minimal`, one 1440 px screenshot at JPEG q 0.65, ≤ 2.5 K prompt tokens, no
research. Anything else is a different benchmark and says so in its table.

## The trace

Every AI request carries a `LatencyTrace`:

```
request_id, trigger,
t_shortcut, t_capture_done, t_snapshot_ready, t_retrieval_done, t_prompt_built,
t_request_sent, t_response_headers, t_first_token, t_first_paint, t_done,
image_bytes, image_px, prompt_tokens, provider_id, model
```

Rust owns the timestamps it can observe — `t_shortcut` from `ShortcutManager`
(`shortcut.triggered.monoMs`), `t_capture_done` and the reply moment from the snapshot builder
(`ContextSnapshot.trace`), `t_request_sent`, `t_response_headers`, `t_first_token` from the AI
manager around the provider adapter — and TS owns `t_snapshot_ready`, `t_retrieval_done`,
`t_prompt_built`, `t_first_paint` (a `requestAnimationFrame` after the first draft) and `t_done`.
Both sides use monotonic clocks (Rust: `crate::clock::mono_ms`, ms since the process started; TS:
`performance.now()`); TS reports its stamps as **offsets relative to the `context_build_snapshot`
reply** (`TraceStamps` on the request, plus a late `ai_report_trace` for first paint / done) and adds
the reply's IPC cost it measured (round trip minus the native build time, the trace's `ipcMs`), so
the halves merge without wall-clock skew (`bluey_core::latency::merge`). Asks without a native
snapshot anchor on the request's arrival. `t_shortcut` is the trigger moment: the global shortcut
keydown when there was one, else the moment the HUD submitted the question. The merged trace is
persisted with the `ai_requests` metrics record (column `trace`, next to `ttft_ms`) and emitted as
`ai.trace` when developer mode is on (or in `dev-tools` / debug builds); the dev overlay
(Settings → Advanced → *Fast path*) shows p50 / p95 per stage over the last 50 requests, cumulative
from the trigger, nearest-rank, never the mean. Extra fields the brief did not list: `ipcMs`;
`imagePx` is the screenshot's long edge; `promptTokens` is the provider's input count when it
reports one, else the engine's estimate.

## The bench

```
bun run bench:fastpath --iterations 30 --provider mock      # local stages only, deterministic
bun run bench:fastpath --iterations 30 --provider gemini    # headline number, needs a key
```

`dev_bench_fast_path { iterations, provider, fixture? }` (developer mode or a `dev-tools` build)
drives the real native path — trigger moment → capture → snapshot assembly → a request carrying the
screenshot → provider → first token — `iterations` times and returns the percentile table
(`BenchReport`, with the markdown to paste). The capture is **live** by default (the numbers are
claimed for the Mac it runs on); `--fixture tests/fixtures/screens/general-1440.jpg.b64` stands a
synthetic 1440 × 900 IDE-like screen in where ScreenCaptureKit cannot run (CI, no screen
permission) or when two runs must see the identical frame — fixture frames skip OCR because the
helper never saw them. `bun run bench:fastpath` launches the app with `--features dev-tools` and
`BLUEY_BENCH_FASTPATH=1` (plus `BLUEY_BENCH_ITERATIONS` / `_PROVIDER` / `_FIXTURE` / `_OUT`); the
app boots, benches after `finish_boot`, prints the table, writes the JSON report (`.bench/`) and
exits (0 clean, 2 some runs failed, 3 could not run); `--ci` fails when the local total p50 is
above the runner threshold. What the bench cannot measure is the WebView's own work — the
TypeScript fuse / budget / prompt build and the first paint — those come from the traces of real
⌘↵ presses in the dev overlay; the bench's `prompt built` row is the Rust stand-in prompt (the same
image payload, no retrieval). Rules:

* 30 iterations, the first 3 discarded (cold caches), p50 and p95 reported, never the mean.
* Same fixture screen, same mode (General), same model for every run in a comparison.
* Run on the Mac that the numbers are claimed for; CI runs the mock provider only and treats the
  local total as an *informative* threshold (≤ 400 ms p50 on the runner — runners are slow and
  noisy), never as a gate on network numbers.
* Every fast-path PR pastes this table in its description, before and after:

| Stage | before p50 / p95 | after p50 / p95 |
|---|---|---|
| capture | | |
| snapshot ready (incl. OCR/AX where still awaited) | | |
| retrieval | | |
| prompt built | | |
| request sent (local total) | | |
| response headers | | |
| first token | | |
| first paint | | |
| image bytes / prompt tokens | | |

## Baseline

**Not yet measured.** PR 4a built the trace and the bench in a sandbox without macOS; the first
table comes from the owner's Mac — run `bun run bench:fastpath --iterations 30 --provider mock`
(local stages) and, with a Gemini key, `--provider gemini`, then paste both tables here under this
heading with the commit and the machine. Until then the structural facts below describe the
baseline the bench will measure (file:line at `49485b5`):

* the snapshot awaits OCR and AX (`src-tauri/src/context/mod.rs:77-78`; helper timeouts capture
  3 s / OCR 5 s / AX 1 s, `sidecar/mod.rs:400-410`; Vision default level `accurate`);
* retrieval runs after the snapshot and embeds the query over the network first
  (`src/ai/engine.ts:268-280`, `src-tauri/src/documents/mod.rs:137-161`);
* frames are 1600 px, JPEG q 0.8, no `mediaResolution` hint (`capture/mod.rs:125`,
  `CaptureTypes.swift`, `bluey-protocols/src/gemini.rs`);
* one `reqwest::Client` without keep-alive tuning or warm-up (`app/mod.rs:199`);
* `ttft_ms` is the only end-to-end-ish metric today (`ai/mod.rs:363-388`).

## Image pipeline

* Active window when one is focused, else the active display; the HUD is excluded.
* 1440 px long edge (device pixels after Retina scaling), JPEG q 0.65, 4:2:0, alpha stripped, no
  metadata, target ≤ 250 KB. *Screen detail* High → 1600–1800 px plus the provider's
  high-resolution hint.
* Legibility guard (`scripts/test-helper.sh` / Swift tests): Vision OCR over the fixture screens
  must agree ≥ 98 % (token level) between the q 0.8 and q 0.65 encodings; otherwise raise quality.
* WebP is used only if ImageIO on macOS 14 can encode it — see the verification table below.

### Provider image and caching facts

Every value below is dated. Re-verify on the day you change the codec; never substitute memory.

Verified 2026-09-11 from the providers' own documentation pages (URLs, quotes and the full
UNVERIFIED list: `docs/reference/verification-2026-09-11/image-and-caching.md`; the project's
`gemini-api-sept-2026.md` agrees with today's pages).

| Provider | Fact | Value (2026-09-11) | Consequence |
|---|---|---|---|
| Gemini | where the hint lives | per-Part `mediaResolution: {level: …}` as a **sibling of `inlineData`** (v1beta, "experimental", Gemini 3 only); or global `generationConfig.mediaResolution` (bare enum); per-part wins | codec adds the per-part field next to the image; global as fallback |
| Gemini | `Level` enum | `MEDIA_RESOLUTION_UNSPECIFIED`, `_LOW`, `_MEDIUM`, `_HIGH`, `_ULTRA_HIGH` (ultra per-part only) | |
| Gemini | tokens per image, "Gemini 3 models" table | unspecified (default) **1120** = high 1120 · low **280** · medium **560** · ultra_high 2240 | omitting the hint costs the same as HIGH — **always send one**. Per-model numbers for `gemini-3.8-flash` / `gemini-3.5-flash-lite` are unpublished → `countTokens` sweep (VERIFY). The API-reference enum comments (64/256) are stale 2.5 numbers |
| Gemini | image input | `image/png`, `image/jpeg`, `image/webp`, `image/heic`, `image/heif`; inline request ≤ 20 MB (another page says 100 MB — design to 20) | irrelevant at ≤ 250 KB frames |
| Gemini | `thinkingLevel` | `gemini-3.8-flash`: `low` · `medium` (default) · `high` — **`minimal` returns an error**; `gemini-3.5-flash-lite`: `minimal` (default) · `low` · `medium` · `high`. Thinking cannot be switched off; `maxOutputTokens` counts thought tokens | the fast role stays on Flash-Lite with `minimal`; never send `minimal` to 3.8-flash; lower the level rather than truncating |
| Gemini | implicit caching | on by default; **minimum 4,096 input tokens** for 3.8 / 3.7 / 3.6 / 3.5 Flash and 3.1 Pro (2,048 for 2.5); Flash-Lite threshold unpublished (VERIFY empirically); hits in `usageMetadata.cachedContentTokenCount` | a ≤ 2.5 K fast-path prompt is **never implicitly cached** — the fast path wins by being small; `Balanced` / `Deep` prompts benefit |
| Gemini | explicit caching | `generateContent` only; `POST /v1beta/cachedContents` (`model`, `contents`, `systemInstruction`, `ttl`, default 1 h) → `cachedContent` on the request; 3.8-flash $0.075 / 1 M cached tokens + $0.50 / 1 M tokens / hour storage; flash-lite $0.03 + $1.00 / hour, not on the free tier | viable for mode + documents once that prefix is ≥ 4,096 tokens; the minimum for explicit caches is "varies by model" (VERIFY) |
| Gemini | warm-up call | `GET https://generativelanguage.googleapis.com/v1beta/models/{model}` (empty body); `x-goog-api-key` header is the documented form everywhere except this endpoint's sample (VERIFY once) | `AiManager::warm` uses `models.get` on the fast model |
| OpenAI (Codex) | `input_image` | `{type: "input_image", image_url: "data:…;base64,…", detail: high \| low \| auto \| original}`; default `auto`; ≤ 30,000 patches per image after resize or **rejected**; 512 MB payload | |
| OpenAI (Codex) | image tokens on current models | patch-based: `ceil(w/32) × ceil(h/32)` patches, shrunk to the detail budget, **× 1.2** for `gpt-6-astra`, `gpt-5.6-*`, `gpt-5.5`, `gpt-5.4`; on `gpt-6-astra` / `gpt-5.6-*` **`auto` = `original`** (full resolution: a 2560×1600 Retina frame ≈ 4,800 tokens, derived); `high` ≤ 2,500 patches (≈ ≤ 3,000 tokens; a 1440×900 frame ≈ 1,566, derived); `low` fits 512×512 (≤ 308 tokens, derived). The 85-token `low` belongs to legacy gpt-4o / 4.1 | send `detail: high` explicitly, never the default; `low` is both illegible at 1440 px and something the Codex CLI itself refuses to send |
| OpenAI (Codex) | prompt caching | on by default; minimum **1,024** tokens (GPT-5.6+); hits `usage.input_tokens_details.cached_tokens`; `prompt_cache_key` is for accounting (the CLI sets it to the session id); TTL 30 min default; images in the prefix are cacheable | static prefix first, screenshot last |
| Anthropic | image tokens | **⌈w/28⌉ × ⌈h/28⌉**; standard tier (all but Claude 4.7+): max 1,568 px long edge / 1,568 tokens; high-resolution tier (Claude 4.7+): 2,576 px / 4,784 tokens, downscaled automatically; JPEG/PNG/GIF/WebP; ≤ 10 MB per image; "images before text" | a 1440×900 frame is 1,716 tokens on a high-res model, 1,560 after the standard tier's downscale — pre-size to ≤ 1,568 px when cost matters |
| Anthropic | prompt caching | `cache_control` on system blocks is documented; minimum **512** tokens (Fable 5.1, Mythos 5.1, Opus 5, Fable 5, Mythos 5), **1,024** (Opus 4.8, Sonnet 5, Sonnet 4.6/4.5, Opus 4.1/4, Sonnet 4), **2,048** (Mythos Preview, Opus 4.7, Haiku 3.5), **4,096** (Opus 4.6/4.5, Haiku 4.5); 5 min default, `ttl: "1h"` at 2× write; 4 breakpoints; hits `usage.cache_read_input_tokens` | breakpoints after system and after documents work at fast-path sizes on current models |
| macOS 14 | WebP encoding via ImageIO | `UTType.webP` (`org.webmproject.webp`) exists since macOS 11 for *decoding*; Apple publishes no encode list; third-party reports mostly say `CGImageDestination` cannot write WebP (2020, 2021, 2026-07) with one 2024 counter-report — **UNVERIFIED** | owner runs on a macOS 14 box: `echo 'import ImageIO; print((CGImageDestinationCopyTypeIdentifiers() as! [String]).contains("org.webmproject.webp"))' > /tmp/w.swift && swift /tmp/w.swift` — `false` → JPEG stays |

## OCR policy

OCR is kept and taken off the critical path (ADR 0010 §3): `context_build_snapshot` returns when
the frame is encoded; OCR (Vision `fast`) and a late accessibility result arrive as
`context.enriched` and are attached to the saved response, the FTS index and the next turn. The
*Screen input* setting (Image · Image + text · Text only) decides what the request carries;
*Text only* awaits OCR (5 s helper timeout) by choice. For follow-up turns on an unchanged screen
hash, the engine sends the OCR text instead of the image.

## Connection warm-up

`AiManager::warm(provider_id)` sends the cheapest authenticated request for the provider (a
model `GET`) at boot, when the HUD becomes visible, when listening starts, and every 45 s while
the HUD is visible or listening. Never while idle in the tray. `t_response_headers −
t_request_sent` in the trace shows the connect cost disappearing; the log line
`warmed <provider>` shows the cadence.

## Prompt prefix stability

Order: identity + safety → mode instructions + schema → documents / personal instructions in a
deterministic order → session memory → transcript → screen → task line. No timestamps, request
ids, or per-request text before the transcript. Audit checklist for `PromptBuilder` /
`SECTION_ORDER`: (1) diff two consecutive prompts for the same mode — the first N tokens must be
byte-identical; (2) documents sorted by id, not by score, inside the stable prefix; (3) the mode
schema serialised once, canonically; (4) Anthropic `cache_control` breakpoints after the system
blocks and after documents; (5) for a Claude subscription account, `system[]` carries only the
Claude Code billing and identity blocks (`docs/PROVIDER_ACCOUNTS.md`) and Bluey's prompt is the
first mid-conversation `role: "system"` message — so it still leads the cacheable prefix right after
them, and the `cache_control` breakpoint moves onto that message.

## Open items (resolve before the PR that depends on them)

| # | Question | Status |
|---|---|---|
| 5 | Gemini `mediaResolution` enum values and token counts for `gemini-3.8-flash` / `gemini-3.5-flash-lite`; implicit-caching thresholds on 3.x Flash | resolved at family level (enum, placement, 280/560/1120/2240; implicit caching from 4,096 tokens); per-model token counts and the Flash-Lite caching threshold need one `countTokens` / cache-hit sweep on a Mac with a key |
| 6 | WebP encoding via ImageIO on macOS 14 | unresolved by documentation; owner runs the one-line check in the table above — JPEG until it prints `true` |
| 7 | Structured output cost on the fast path (`responseJsonSchema` vs markdown TTFT, seven mode fixtures × 5 runs) | open — run with the bench in PR 4b/5, record the decision here |
| 8 | Rate-limit UX copy ("Claude 5h window resets at 14:32 — using Gemini meanwhile") | open — decided in PR 2 with the Accounts UI |
