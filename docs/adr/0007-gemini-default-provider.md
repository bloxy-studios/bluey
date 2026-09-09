# ADR 0007 — Google Gemini is the default provider, driven by one AI Studio key

**Status:** accepted · **Date:** 2026-09-08

## Context
Bluey shipped with Microsoft Foundry / Azure OpenAI as the implied default and Claude for deep
research, which meant two vendors, two keys and a Foundry resource before the first answer.
Google AI Studio offers a single key that covers every capability Bluey needs — chat and
vision (`gemini-3.8-flash`, `gemini-3.5-flash-lite`), live and batch transcription
(`gemini-3.5-transcribe-live`, `gemini-3.5-transcribe`), embeddings (`gemini-embedding-2`) and
agentic research (function calling on `gemini-3.8-flash`) — with a free tier that is enough to
evaluate the product. The verified API facts Bluey codes against live in
`docs/reference/gemini-api-sept-2026.md`; that document, not memory, is the source of truth.

## Decision
1. **Gemini is the default provider for every AI feature.** A fresh install with a
   `GEMINI_API_KEY` (alias `GOOGLE_API_KEY`) in `.env`, or a key pasted in onboarding, gets
   working chat, screenshot vision, transcription, embeddings and deep research without any
   other configuration.
2. **The other providers stay first-class alternates.** Microsoft Foundry / Azure OpenAI,
   Anthropic and OpenAI-compatible endpoints keep their adapters unchanged; switching is one
   environment variable (`BLUEY_AI_PROVIDER=gemini|azure-foundry|anthropic|openai`) or one click
   in Settings → AI ("Default provider"). Mixed setups (e.g. Gemini for answers, Foundry for
   embeddings) remain possible through per-role assignments.
3. **Presets, not hard-coded models.** `bluey_core::presets` is the single table of reserved
   provider ids (`gemini`, `azure-foundry`, `anthropic`, `openai`), display names and the
   recommended model per role. `ai_apply_provider_presets { providerId, overwrite }` and the
   `.env` import both apply it; `BLUEY_MODEL_*` variables override single roles.
4. **The key never leaves Rust.** Like every provider key it lives in the macOS Keychain under
   `provider:gemini:api_key` and is used only by the Rust adapter (`x-goog-api-key` header,
   never `?key=` on HTTP) and, per job, by the Bun research sidecar's environment
   (`GEMINI_API_KEY`). The Live API WebSocket URL is the one place the key travels in the query
   string; it is redacted from every log line. The WebView bundle does not contain `@google/genai`
   and never talks to Gemini directly (ADR 0001).
5. **`.env` import is Keychain-first and never undoes Settings edits.** Keys found in the
   environment are copied into the Keychain only when the Keychain has no entry for that
   provider, unless `BLUEY_ENV_OVERRIDES_KEYCHAIN=1`. The import logs
   `imported api key for provider <id>` and nothing else — never values, never lengths. The
   nominated provider is remembered as `ai.bootstrapProvider`: an unchanged nomination only
   fills unassigned roles, a changed `BLUEY_AI_PROVIDER` re-applies that provider's presets,
   and the knobs (`BLUEY_EMBEDDING_DIMENSIONS`, `BLUEY_TRANSCRIPTION_PROVIDER`,
   `RESEARCH_BACKEND`) apply on the first import or a changed nomination only. Existing
   providers keep their enabled state and base URL (unless the base URL is set in the
   environment). `bootstrapProvider` is also written by onboarding and the Settings → AI
   default-provider select — it records whose presets were last applied.
   Sidecars run with a cleared environment so the keys `.env` loads into Bluey's process reach
   only the child that needs them.
6. **Gemini 3.x request rules are enforced in the codec**, not scattered in adapters:
   `thinkingLevel` (never `thinkingBudget`, `temperature`, `topP`, `topK` or `candidateCount` on
   3.x models), roles `user`/`model` with `systemInstruction`, structured output through
   `responseJsonSchema` with the top-level `$schema` stripped, and the error table
   (`config.api_key_invalid`, `config.model_not_found`, `network.http_429` with `retryAfterMs`
   and daily-quota detection, `ai.blocked_<reason>`).

## Consequences
* Onboarding asks for one key. Research, transcription and embeddings light up with it; the
  Claude research backend and Foundry Voice Live remain opt-in (`RESEARCH_BACKEND=claude`,
  `BLUEY_TRANSCRIPTION_PROVIDER=cloud_realtime`).
* The research sidecar ships as a **lite** binary (Gemini only) by default; the **full** binary
  with the embedded Claude CLI is built only when the Claude backend is selected.
* Adding a fifth provider kind means one adapter, one preset row and one TS mirror entry — the
  router, settings and UI stay untouched.

## Explicitly out of scope
Gemini Interactions API / managed agents, Google Search grounding, image or speech generation,
Vertex AI authentication, and using Gemini's OpenAI-compatible endpoint as the primary path.
