# Testing

## Automated

| Layer | Tool | Command | Covers |
|---|---|---|---|
| Rust core | `cargo test -p bluey-core` | `bun run test:rust` | state machine matrix, event names ⇄ TS list, model routing & fallbacks, token budgeting, snapshot trimming, adapters, built-in modes, shortcut parsing/conflicts, session rules, text utils, provider presets + `.env` import planning (re-apply on a changed nomination, knobs on first import only, keyless-nomination fallback) |
| Rust storage | `cargo test -p bluey-storage` | `bun run test:rust` | migrations, every repository, FTS sync & session search, document parsing (PDF/DOCX/TXT/MD), chunking, keyword/semantic retrieval (vectors of another size are skipped), embedding-model tracking + stale detection, retention really deleting |
| Rust protocols | `cargo test -p bluey-protocols` | `bun run check:rust` | SSE parsing, provider request/chunk codecs (Gemini `generateContent` bodies + thinking policy + error table + embeddings + models + Live frames, Azure v1 + legacy, OpenAI-compatible, Anthropic incl. `input_json_delta`), Exa/Firecrawl models, realtime transcription messages, batch-transcription bodies/Files API/`audioTranscription` parsing + recording-import segmenting, sidecar envelopes + helper/agent mappers, panel geometry, Clerk key derivation |
| Rust app crate | `cargo clippy --target aarch64-apple-darwin --no-default-features --features dev-tools --all-targets -- -D warnings` (Linux, type-check only) · `cd src-tauri && cargo test --features dev-tools` (macOS) | `bun run check:rust --darwin` | every module compiles for macOS incl. its unit tests (audio routing, transcription dedupe/rotation constants, Gemini adapter backoff, env import) |
| TypeScript unit | Vitest | `bun run test` | context fusion, budget, intent, prompt builder, code-fence buffering, generation gate, optimizer, structured parsing, classifier, speaker labels, research router + privacy scrubbing, summaries, stores |
| TypeScript integration | Vitest + fake transport | `bun run test` | capture → context → request, transcript → detection → prepare/take, stale-request protection, retrieval scoping, session → summary, command-surface parity |
| UI | Vitest + Testing Library + MockTransport | `bun run test` | HUD per app state (incl. live transcript strip, error pill + recovery, research progress, prepared-response take), proactive loop, error toasts, sessions tab, My Context tab, AI tab (default provider, presets, role model lists, research backend), Appearance / Privacy / Keybinds toggles, mode editor, response rendering/copy, onboarding flow incl. Connect Gemini |
| Agent sidecar | Vitest | `cd sidecars/agent && bun run test` | protocol against both mock backends (gemini / claude), Gemini function-calling loop (ids, thought signatures, JSON report, usage, cancel, max turns, blocked, error mapping), `RESEARCH_BACKEND` config, tool mapping (mocked fetch), allow-lists, cancellation |
| Swift helper | `swift test` (macOS) | `bash scripts/test-helper.sh` | dHash/VAD/envelope/ordering logic |
| Type/lint | tsc, ESLint, clippy, rustfmt | `bun run typecheck && bun run lint && bun run check:rust` | — |

Fixtures live in `tests/fixtures/<mode>/` (transcript, OCR, snapshot, mode, expected shape) so
no real meeting is needed to exercise the pipeline.

## Native test harness (macOS)
`tests/native/README.md` explains how to run `bluey-helper` from a terminal and feed JSON-Lines
requests (`tests/native/requests/*.jsonl`) to verify screen capture, OCR, microphone, system
audio, accessibility and observation deterministically.

## Manual QA checklist
**Authentication** — login · logout · session restore after relaunch · missing publishable key screen.
**Permissions** — grant · deny · revoke while running (audio stops, repair flow) · retry · Open System Settings links.
**Screen** — single monitor · multiple monitors · Retina scaling · fullscreen app · Spaces · mirrored/disconnected display · region capture · HUD excluded from captures.
**Audio** — microphone · system audio · headphones · Bluetooth device · device disconnect mid-session · pause/resume · levels.
**Panel** — move (⌘ arrows) · drag · resize · hide/show (⌘\) · always-on-top over fullscreen apps · remembers position per display · never off-screen · opacity/width settings · Privacy mode hides it from a screen share (Zoom/Meet/QuickTime) and the tooltip reports the state honestly.
**Shortcuts** — defaults · remap · conflict warning · disabled shortcut · registration failure message.
**AI** — fast answer · streaming · structured sections · code copy · vision (screen with little text) · failure · timeout · cancellation (new ⌘↵ while streaming) · offline banner · provider test connection.
**Modes** — every built-in mode with its fixture scenario · custom mode create/duplicate/edit/delete/set default/set active · reset built-in.
**Documents** — upload PDF/DOCX/TXT/MD · parse errors · retrieval shows in responses · delete.
**Session** — start · pause · resume · end · timeline events clickable · notes · summary · search · export · delete.
**Privacy Center** — toggles apply immediately · one-click disable all capture · data deletion counts drop to zero · reset Bluey returns to onboarding.
**Menu bar** — state text updates · every item works · quit cleans up helper and agent processes.

## Gemini smoke tests (real key, macOS)

With only `GEMINI_API_KEY` in `.env` (no other provider):

1. **Import** — first launch logs `imported api key for provider gemini` and nothing else; Settings → AI shows *Google Gemini* as the default provider with every role on its recommended model; the Keychain has `provider:gemini:api_key`; `.env` can be emptied afterwards.
2. **Chat** — ⌘↵ on a screen with text: first answer streams from `gemini-3.8-flash`; the dev overlay shows TTFT / tokens; no `temperature`/`topP` in the request (Gemini 3.x rules).
3. **Vision** — a screenshot with little text (a chart) yields an answer that references the image.
4. **Reasoning** — a system-design question in *System Design* mode uses `thinkingLevel: high` (longer TTFT, deeper answer).
5. **Structured output** — coding mode returns sections + a code block (JSON schema path).
6. **Embeddings** — add a résumé in Settings → Context; the document shows *Indexed · Embedded*; a question about it retrieves the right excerpt. Change *Embedding size* to 1536 → the document re-embeds (log: `re-embedding documents…`) and retrieval still works.
7. **Live transcription** — start listening with mic + system audio: partials then finals appear in the transcript strip with speaker labels; leave it running past 9 min 30 s → the log shows the rotation and no final is duplicated or lost; remove the key → the next start shows the *stt_fallback* toast and Apple Speech takes over.
8. **Batch transcription** — Settings → Sessions → *Import recording…* with a WAV/MP3: a new completed session “Imported · <file>” opens with a *Recording imported* timeline event and a Transcript section labelled *Speaker 1 / Speaker 2*; *Add recording* on an existing session appends to it; an `.m4a` is refused with a clear message; a file above 14 MB goes through the Files API (log: `transcribing recording … inline=false`) and the upload is deleted right after.
9. **Deep research** — a question that needs fresh facts with Exa/Firecrawl keys present: the pill shows *Researching*, the placeholder lists lookups, *Skip research* cancels the sidecar job, the answer carries citations; `RESEARCH_BACKEND=gemini` is the lite sidecar with `GEMINI_API_KEY` only in its environment.
10. **Errors** — a wrong key → *API key rejected* with the AI Studio link; a bogus model id → *Model not available*; exhaust the free tier → *Daily quota reached* / *Rate limited* with the retry delay.
11. **Switch providers** — Settings → AI → *Default AI provider* → Foundry: every role moves to the Foundry presets, the Foundry key field appears; switch back to Gemini with one click. `BLUEY_AI_PROVIDER=anthropic` in `.env` does the same at boot.
