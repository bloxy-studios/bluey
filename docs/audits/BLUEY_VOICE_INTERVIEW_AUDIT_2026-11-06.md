# Bluey audit — live voice interviews, providers and plan models (2026-11-06)

Scope: everything between "the interviewer speaks" and "the answer is on screen", plus the AI
providers and model catalogs that path depends on. Three read-only audits ran in parallel (voice
pipeline; question → answer path; provider adapters). Every finding below was checked against
the code before it was fixed. The 2026-09-28 audit was used to skip items already fixed.

Status legend: **Fixed** (code + tests in this change) · **Mitigated** (the user-facing failure
is covered by another fix; the root cause remains) · **Open** (not changed; reason given).

## 1. What was most likely causing "errors with some providers"

| # | Severity | Finding | Status |
|---|---|---|---|
| P1 | Critical | **Claude (API key and Claude account) rejected plain answers.** Every request carried `temperature`; Opus 4.7+, Opus 5/5.5, Sonnet 5 and Fable answer it with HTTP 400 (Sonnet/Haiku 5.5 reject non-default values). Plain interview answers have reasoning `none`, so the thinking path that stripped `temperature` never ran. The Claude account's probe sent no temperature, so connecting worked and real questions failed. | **Fixed** — sampling knobs are never sent to Anthropic, Claude Code or Claude-on-Antigravity (`anthropic.rs`, `claude_code::apply_thinking`, `antigravity::normalise_request`). |
| P2 | High | **Adaptive thinking sent to models that don't have it** (Sonnet 4.5, Opus 4.5, Opus 4.1 …): every coding/design/summary ask on them was a 400. | **Fixed** — `claude_code::supports_adaptive` gates on the model generation (4.6+, Haiku 5+, Fable). |
| P3 | High | **Google AI (Antigravity) answers cut off or empty**: Claude models got a thinking budget of `min(budget, maxOutputTokens − 1)`, leaving ~1 token for the answer; Gemini 3 models got no thinking allowance. | **Fixed** — the answer's cap now sits on top of the thinking budget; Gemini models get the same allowance as the direct Gemini adapter. |
| P4 | High | **No timeout on OpenAI-compatible, Azure and Anthropic streams**: one stalled answer kept the HUD on "Thinking" and blocked every later question until Esc. Gemini/ChatGPT waited 90–120 s. | **Fixed** — `AiManager::drive_provider` now enforces a first-token deadline (12 s ultra-fast, 15 s fast, 45 s balanced, 150 s deep — covering the adapters' own retry waits) and a 45 s idle limit once text flows. |
| P5 | Medium | Cut-off or failed streams reported as finished: OpenAI-style mid-stream `{"error":…}` frames ignored, `content_filter`/`error` finish reasons → "stop", a stream ending with no terminal event → "stop"; Anthropic `refusal` and a missing `message_stop` → "stop". | **Fixed** — these are now errors (and fail over when nothing had streamed). A partial answer that ends without a marker is kept. |
| P6 | Medium | OpenAI `429 insufficient_quota` (no credits) shown as a temporary rate limit that never clears. | **Fixed** — `config.quota_exhausted`, with the provider settings as the recovery. |
| P7 | Medium | ChatGPT: "The 'X' model is not supported when using Codex with a ChatGPT account" read as a generic 400 and triggered the process-wide instructions fallback. | **Fixed** — mapped to `config.model_not_found` (catalog is refreshed in the background); the instructions fallback only follows an unexplained 400. |
| P8 | Medium | DeepSeek's "response_format … is unavailable" not recognised, so structured asks always failed there. | **Fixed** |
| P9 | Medium | OpenAI-style reasoning models (gpt-5+, o-series, also via Azure) spend `max_completion_tokens` on reasoning first, so short answer caps came back empty (`length`). | **Fixed** — headroom by effort (2–16 K) on top of the answer's cap. |

## 2. Failover — a question never ends in an error while another provider can answer

Before: when the chosen provider failed, the HUD showed the error and only *hinted* at the
fallback. Now (`AiManager::run_stream` + `router::failover`): a failure **before the first token**
— network, 5xx/overload, rate or plan limit, refused key, model not offered, quota, refusal, or the
first-token deadline — reruns the same request on the next usable provider (the role's other
assignments first, then the API-key stand-in), up to twice. The WebView gets a second `started`
chunk naming the provider that answered. Failed subscription accounts are moved to their state
(rate-limited, needs re-auth, …) first, so the next question routes around them too. Once text has
streamed a failure stands (no duplicated answers).

## 3. Plan models — fetched, not hard-coded

| Provider | Before | Now |
|---|---|---|
| ChatGPT (Free/Go/Plus/Pro/Team/Business/Enterprise/Edu) | Catalog fetched from `chatgpt.com/backend-api/codex/models` **only at connect or on "Refresh models"**; a plan filter on `available_in_plans` could empty the list (that field does not exist upstream — the backend already answers per account). | Catalog re-read **at every launch** (fresh caches kept) and **whenever a model is refused**; the plan filter only narrows when the catalog names the plan (Team ≡ Business), never to an empty list. |
| Claude Pro/Max | Catalog at connect; curated fallback list missing Opus 5.5 / Sonnet 5.5 / Haiku 5.5. | Same refresh rules; curated list leads with the current models. |
| Google AI Pro/Ultra (Antigravity) | Catalog at connect. | Same refresh rules. |
| Gemini / Anthropic API keys | Roles pointed at hard-coded preset names; a retired or unavailable name failed every request. | Presets are checked against the key's **live model list** before they are assigned; a missing one becomes the newest listed model of the same line (flash / flash-lite / embedding; sonnet / haiku / opus). At launch, roles a preset assigned are re-checked; models you chose yourself are never changed. Anthropic's list now asks for all models (the default page is 20). |

**Instant responses.** ChatGPT's catalog advertises a **Fast tier** per model (`service_tiers:
[{id: "priority"}]`, the Codex CLI's `/fast` mode). Live answers (ultra-fast/fast budgets) now ask
for it on models whose catalog offers it; if the backend refuses it once, Bluey drops it for the
rest of the run. Note: Fast mode draws plan usage faster (OpenAI's own client documents this).
Also: the first ChatGPT answer after launch no longer waits for a catalog round-trip (the Codex
template is only fetched after a refusal); Claude models that think by default (Opus/Sonnet 5+,
Haiku 5.5, Fable) get effort `low` on plain answers instead of their default depth, with thinking
headroom added to `max_tokens`.

## 4. Live voice pipeline

| # | Severity | Finding | Status |
|---|---|---|---|
| V1 | High | Gemini Live reports a bad key (1007), unknown model (1008) and **spent quota (1011 "exceeded your current quota")** as WebSocket **close frames**; Bluey never read them, so these reconnected **forever** and never fell back to Apple Speech — no transcript for the rest of the interview. | **Fixed** — `map_live_close`; key/model/quota/policy closes end the session and listening moves to Apple Speech. Reconnects are capped at 8 attempts per outage; a source that gives up twice without a transcript moves to Apple Speech instead of cooling down again. |
| V2 | High | A system-audio stream error (display replug, clamshell, re-consent prompt) or an empty input-device list **stopped the whole run, microphone included**. | **Fixed** (Swift) — only the failed source stops; system audio restarts with backoff (1, 2, 5, then every 10 s); a vanished mic is suspended and resumes on the next device. A Stop you click in the macOS recording indicator is honoured (not retried), and a revoked Screen Recording permission ends the retries with one error. |
| V3 | High | **Audio spoken during any reconnect was thrown away** — a question asked during a network blip was never transcribed, so never answered. | **Fixed** — audio sent since the last final and audio arriving while reconnecting (~10 s) is replayed to the new socket (Gemini Live and Voice Live); dedupe is armed for the replay. |
| V4 | Medium | A failed session rotation (at 9 min 30 s) dropped the still-healthy socket. | **Fixed** — the old socket keeps serving; rotation retries every 5 s. |
| V5 | Medium | The read watchdog reconnected every 15 s during typing or music (energy VAD "speech", no transcript), each reconnect losing audio. | **Fixed** — the watchdog pings first; only an unanswered ping (5 s) counts as stalled. |
| V6 | Medium | Apple Speech died permanently after one "unavailable" or five errors. | **Fixed** (Swift) — restarts on availability change; backoff retries (2, 5, 10, 30 s). |
| V7 | Medium | Mic and system audio use separate sample-count clocks; after a capture gap one source's timestamps lag, which can push the interviewer's earlier lines out of the 45 s classifier window. | **Open** — needs a shared host clock in the helper and a device test. |

## 5. Question → answer path (frontend)

| # | Severity | Finding | Status |
|---|---|---|---|
| A1 | High | Coding answers painted nothing until the whole fenced solution had streamed. | **Fixed** — an open fence renders progressively (Copy waits for the finished block). |
| A2 | High | Heard questions in System Design / Coding modes used Deep/Balanced budgets and the Reasoning role: 5–30 s to first token. | **Fixed** — live suggestions (prepare) are capped at the fast budget with no deep reasoning (coding keeps light) and never use the reasoning/research roles. Your own ⌘↵ asks and **Regenerate** keep the mode's full depth. |
| A3 | Medium-high | A question split by a pause: the fragment was answered and the real question dropped by the cooldown. | **Fixed** — the continuation (same voice, ≤ 3 s) merges and replaces the fragment's suggestion; ≤ 5-word fragments heard alone are listed but not answered. |
| A4 | Medium | In screen modes, little OCR text (a video call) sent a heard question as an image to the slower vision role. | **Fixed** — heard questions use the image only when they refer to the screen or a problem is visibly on it. |
| A5 | Medium | A question queued behind a long answer was dropped as stale after 20 s; a newer question never replaced a suggestion that had not produced a token yet. | **Fixed** |
| A6 | Medium | The optional fast-model classification delayed detection with no deadline and could reorder questions. | **Fixed** — the heuristic detection is announced at once; the model gets 700 ms and can only withdraw it. |
| A7 | Medium | ⌘⇧↵ during a streaming live suggestion cancelled it and started over. | **Fixed** — the live suggestion is kept and revealed. |
| A8 | Low | A cancel issued before Rust registered the request was lost; a throwing `prepare()` left the live turn spinning. | **Fixed** |
| A9 | High (opt-in) | With embeddings on, retrieval waits for the query embedding with no deadline. | **Open** — embeddings are off by default; recommended: race the embedding against ~150 ms and fall back to keyword search. |
| A10 | Medium | Live suggestions in screen modes capture the screen (150–1 150 ms). | **Open** — latency only; recommended: reuse the last frame/OCR for heard questions. |

## 6. Still open (recommended next)

* **Codex client version — highest priority for "fetch my plan's models".** Bluey pins
  `0.154.0` (captured 2026-09-11); the Codex CLI is at **0.167.0** (2026-11-06, 2 582 commits
  later). The catalog is requested with `client_version=0.154.0`, and the backend leaves out
  models whose minimum client version is newer — so the newest models of a ChatGPT plan can be
  missing from Bluey's list, and eventually every model can be "not supported". Bump it after a
  fresh capture of the 0.167 wire format (docs/PROVIDER_ACCOUNTS.md runbook, `fingerprints:*`) —
  not changed here, because the request shape may have moved with it.
* Generic OpenAI-compatible servers that only know `max_tokens`; Azure deployments whose names hide
  a reasoning model still get `temperature` (no retry without it yet).
* Claude/ChatGPT accounts do not retry a 529/5xx on the same provider; failover now covers the
  user-facing failure when another provider is configured.
* Fable 5.1 on the Claude account sends `fallbacks` with the `fallback-credit-2026-06-01` beta; the
  public API documents `server-side-fallback-2026-06-01` for that array form — verify with a fresh
  Claude Code capture.
* Claude Sonnet 4.5 (`claude-sonnet-4-5-20250929`, referenced in docs and tests) is deprecated and
  retires on **2026-11-30**; the current Sonnet is `claude-sonnet-5-5`.

## 7. Verification

* Rust (`cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` clean): `bluey-core` 129 tests, `bluey-protocols` 226 tests and the
  app crate (`bluey`) 199 tests pass — new tests cover failover routing and eligibility, plan
  filters, the Fast tier and its refusal, Claude thinking rules and sampling, Antigravity budgets,
  OpenAI stream errors and reasoning headroom, quota mapping, live model replacement, Gemini Live
  close frames, reconnect replay and give-up, and the STT fallback count.
* Swift helper: x86_64 build clean; 71 Swift tests pass.
* Frontend: `tsc` and ESLint clean; full Vitest suite 1 240 passed, 26 skipped, 0 failed (two older tests
  updated to the new contract: live suggestions use the default role in System Design mode, and
  coding drafts stream an open code block).
* **Not verified live**: no request was sent to a real provider, and no hardware scenario (display
  unplug, AirPods disconnect, quota exhaustion, network blip) was exercised on a device. Each fix
  is unit-tested against the documented wire behaviour; a short live run per provider is the
  remaining check.
