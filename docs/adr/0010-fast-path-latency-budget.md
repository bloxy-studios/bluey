# ADR 0010 — The ⌘↵ fast path: a measured latency budget, OCR off the critical path

**Status:** accepted · **Date:** 2026-09-11 · companion: `docs/LATENCY.md`,
`docs/reference/provider-accounts-fast-path-brief.md` §5

## Context
`docs/AI_ARCHITECTURE.md` lists stage targets (capture < 200 ms, OCR < 500 ms, context assembly
< 300 ms, fast first token < 1 s) but nothing measures the path end to end, and the code on `main`
(`49485b5`) spends its time serially:

* `context_build_snapshot` (`src-tauri/src/context/mod.rs:77-78`) does
  `tokio::join!(frontmost, accessibility, capture_and_ocr)`, and `capture_and_ocr` runs capture
  *then* OCR inside one future — the snapshot is not ready until OCR (helper timeout 5 s) and the
  accessibility snapshot (1 s) have both returned. `SnapshotOptions::default()` asks for both, and
  the helper's default Vision level is `accurate` (`OCRService.swift`), although
  `CAPTURE_ARCHITECTURE.md` documents `fast` as the default.
* The TypeScript engine (`src/ai/engine.ts:268-389`) awaits the snapshot, then document retrieval,
  then research, then fuses, budgets and streams. Retrieval does not depend on the screenshot.
* When documents are indexed, `DocumentsManager::retrieve` (`src-tauri/src/documents/mod.rs:137-161`)
  embeds the query over the network (`gemini-embedding-2`) before the main request — one extra
  provider round-trip on the critical path.
* Frames are 1600 px long-edge JPEG at quality 0.8 (`capture/mod.rs:125`, `CaptureTypes.swift`),
  sent inline; the Gemini codec sends no `mediaResolution`, so every image costs the model's
  default image budget. One shared `reqwest::Client` (`app/mod.rs:199`) is never pre-warmed, and
  every `AiChunk` is serialized twice — once for the `Channel`, once for the broadcast bus
  (`ai/mod.rs:394-395`).

The owner's target, "⌘↵ feels instant", was first phrased as 120–500 ms end to end. A cloud
model's own time-to-first-token on the fastest Flash models is 300–800 ms from a warm connection
and outside Bluey's control, so that number cannot be an end-to-end promise. It **is** the right
budget for everything Bluey does before the request leaves the Mac.

## Decision
1. **A budget, measured from the ⌘↵ keydown**, on a warm connection with `gemini-3.5-flash-lite`,
   one 1440 px screenshot and ≤ 2.5 K prompt tokens. Local stages are **blocking gates** for the
   fast-path PRs; network stages are reported, not promised.

   | Stage | p50 | p95 |
   |---|---|---|
   | shortcut → frame encoded (SCK + downscale + JPEG) | ≤ 90 ms | ≤ 150 ms |
   | accessibility snapshot (raced, soft deadline) | ≤ 80 ms or dropped from this request | — |
   | OCR on the critical path | 0 | 0 |
   | retrieval (parallel with capture; keyword + cached embedding only) | ≤ 15 ms | — |
   | fuse + budget + prompt build (TS) | ≤ 15 ms | ≤ 30 ms |
   | IPC + base64 + serialization (image ≤ 250 KB) | ≤ 20 ms | ≤ 40 ms |
   | **local total before the request leaves** | **≤ 150 ms** | **≤ 300 ms** |
   | connect (pre-warmed) + upload | ≤ 60 ms | ≤ 120 ms |
   | provider first token (flash-lite, minimal thinking) | 300–700 ms (measured) | ≤ 1200 ms |
   | first token → first paint | ≤ 16 ms | — |
   | **⌘↵ → first visible token** | **≤ 800 ms** | **≤ 1500 ms** |

2. **Instrumentation before optimisation.** A `LatencyTrace` per request (shortcut, capture done,
   snapshot ready, retrieval done, prompt built, request sent, response headers, first token,
   first paint, done; image bytes/px, prompt tokens, provider, model) is assembled from the Rust
   timestamps and the TS timestamps by `request_id`, persisted with the `ai_requests` metrics
   record, and shown as p50/p95 per stage in the dev overlay (`ai.trace`, dev-only). A bench
   (`bun run bench:fastpath`) runs the local stages deterministically against the mock provider
   and, with a key, the headline number against the real one. Every fast-path PR pastes a
   before/after table (`docs/LATENCY.md`).
3. **OCR stays, but off the critical path.** Vision models read screenshots well enough that OCR
   must not gate the first request; OCR is still the cheapest way to serve text-only models and
   follow-up turns without re-sending an image, to make sessions searchable, to feed the proactive
   classifier and the privacy scrub, and to keep small-text fidelity. So `context_build_snapshot`
   returns as soon as the frame is encoded; the accessibility snapshot is raced with an 80 ms soft
   deadline (its focused-element value and selected text are the high-value part); OCR and a late
   AX result are published afterwards as `context.enriched { snapshotId, ocr, accessibility }`,
   which the engine attaches to the saved response, the FTS index and the *next* turn (an
   unchanged screen hash reuses OCR text instead of the image). The enrichment path uses Vision
   `fast`; `accurate` only when *Screen detail* is High. A new setting *Screen input*
   (Image · Image + text · Text only) covers non-vision models — *Text only* is the one path that
   awaits OCR, by choice.
4. **Readable, not big.** Capture the active window when one is focused (display otherwise, HUD
   excluded); 1440 px long edge (from 1600), JPEG quality 0.65 (from 0.8), 4:2:0, no alpha or
   metadata, ≤ 250 KB; a legibility guard in the helper tests requires ≥ 98 % token agreement
   between Vision OCR over the q 0.8 and q 0.65 encodings of the fixture screens. *Screen detail*
   Auto / Standard / High raises the edge to 1600–1800 px and asks for the provider's
   high-resolution hint for dense text. Provider hints are sent explicitly: Gemini per-part
   `mediaResolution` (medium by default, high for a dense-text bundle-id allow-list — Xcode, VS
   Code, Cursor, JetBrains, terminals, Zed, spreadsheets, browsers on code hosts — or when the user
   chooses High), OpenAI `input_image.detail`, Anthropic ≤ 1568 px. The exact enum names and token
   costs are pinned, with dates, in `docs/LATENCY.md` — never from memory. Inline images never take
   the Files API on the fast path; dHash de-duplication keeps reusing an unchanged frame.
5. **Retrieval in parallel, embedding off the hot path.** `retrieveRelevantContext` starts
   concurrently with `buildNativeSnapshot`. A `RetrievalStrategy::Fast` does FTS5 keyword retrieval
   plus semantic retrieval only when a query embedding is already cached (Rust LRU keyed by the
   normalised query, 256 entries, 30 min). While listening, the last detected question is embedded
   proactively (debounced 2 s, only when embeddings are ready and a library exists) so ⌘↵ / ⌘⇧↵
   hit the cache. `Balanced` / `Deep` requests keep full hybrid retrieval.
6. **Warm connections.** The shared client keeps HTTP/2 connections alive
   (`pool_idle_timeout` 90 s, TCP keep-alive 30 s, HTTP/2 keep-alive 30 s incl. while idle) and
   `AiManager::warm(provider)` issues the cheapest authenticated request (a model `GET`) at boot,
   when the HUD becomes visible, when listening starts and every 45 s while the HUD is visible or
   listening — never more often, never while idle in the tray. Connected subscription accounts
   (ADR 0009) warm their hosts the same way.
7. **Stable prefix first, volatile last.** The prompt is ordered identity + safety → mode
   instructions + schema → documents / personal instructions (deterministic order) → session
   memory → transcript → screen → task line, with no timestamps or ids in the prefix, so Gemini
   implicit caching, OpenAI automatic prompt caching and Anthropic `cache_control` breakpoints
   (after system, after documents) cut prefill. For a Claude subscription account (ADR 0009) the
   `system[]` array holds only the Claude Code billing and identity blocks, and Bluey's own prompt
   follows as the first mid-conversation system message — still the head of the cacheable prefix,
   never a further `system` block. The thresholds bound what caching buys: Gemini 3.x
   Flash caches implicitly only from **4,096** input tokens, so a ≤ 2.5 K fast-path prompt is never
   implicitly cached — the fast path wins by being small, and caching pays on `Balanced` / `Deep`
   prompts and on an explicit `cachedContents` for mode + documents; OpenAI caches from 1,024
   tokens (GPT-5.6+), Anthropic from 512–4,096 depending on the model (`docs/LATENCY.md`).
8. **Start earlier — without capturing anything new.** When the HUD is shown or listening starts
   *and* screen permission is granted *and* smart observation is on, the observer's most recent
   frame (≤ 1.5 s old, unchanged dHash) is reused by ⌘↵ instead of a fresh capture, and retrieval
   for the current transcript question is precomputed. This reorders work the user already opted
   into; nothing is captured silently (`docs/SECURITY.md`).
9. **Fast role tuning and hot-path hygiene.** `UltraFast` / `Fast` use `gemini-3.5-flash-lite`,
   `thinkingLevel: minimal` (the Flash-Lite line is the only current tier that accepts `minimal`;
   `gemini-3.8-flash` returns an error for it, which is why the fast role stays on Flash-Lite),
   `maxOutputTokens ≤ 400`, ≤ 2,500 context tokens and one medium-resolution image; the default ⌘↵ trigger is `Fast`, modes may raise it, and a *Think deeper*
   action re-asks on the reasoning role. Structured output is kept on the fast path only if the
   schema-vs-markdown experiment costs ≤ 150 ms p50 TTFT. `ai.chunk` is no longer mirrored to the
   broadcast bus outside `dev-tools` (the WebView consumes the `Channel`; `ai.completed` stays);
   high-rate `audio.level` gets its own channel so a lagging forwarder cannot drop `app.state`.

## Consequences
* New contract surface: event `context.enriched`, settings *Screen input* and *Screen detail*,
  `RetrievalStrategy::Fast`, `LatencyTrace` on the metrics record, dev command
  `dev_bench_fast_path`, script `bun run bench:fastpath`. Existing event and command names do not
  change; the TS ⇄ Rust parity tests gain the new names on both sides.
* `docs/ARCHITECTURE.md` step 3 ("OCR and accessibility in parallel") and
  `docs/CAPTURE_ARCHITECTURE.md` (OCR default level) are corrected in the PR that changes the
  behaviour; the Swift default level and the documentation must agree.
* The engine's integration tests assert that `buildNativeSnapshot` resolves without OCR and that a
  cold embedding cache issues no embedding call on the fast path; the helper tests gain the JPEG
  legibility guard; `docs/TESTING.md` gains the QA lines (trace in the overlay, OCR arriving after
  the answer starts, *Text only* still works, warm-up cadence visible in the log and absent in the
  tray, q 0.65 legible on Retina and 1× displays).
* Network numbers move with the provider; the ADR fixes the *local* budget and the method, not a
  TTFT the code cannot control.

## Explicitly out of scope
On-device models, changing which provider serves the fast role, routing inline images through the
Files API, and any capture that the user has not already enabled.
