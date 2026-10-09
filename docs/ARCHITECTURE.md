# Bluey Architecture

Bluey is a macOS-first, real-time AI context copilot. It understands what is happening on the
user's computer — screen, focused UI, and conversation — **only through explicitly granted
permissions**, fuses that context with the user's own documents and the active mode, and
prepares concise, contextual help in a floating panel.

```
                                  BLUEY
                                    │
                 ┌──────────────────┼───────────────────┐
                 │                  │                   │
          Native layer        Rust orchestration    Frontend layer
      (Swift helper sidecar)     (Tauri app)          (WebView)
                 │                  │                   │
       ┌────┬────┼────┬────┐        │           ┌───────┼───────┐
     Screen OCR  AX Audio Speech    │          HUD    Modes   Settings
       └────┴────┼────┴────┘        │           └───────┼───────┘
                 │  JSON lines      │                   │ typed API + events
                 └──────────► Context Snapshot ◄────────┘
                                    │
                             Context Fusion  (TS: src/context)
                                    │
                             Relevance / Intent
                                    │
                             Mode Intelligence  (TS: src/modes, src/ai/prompts)
                                    │
                               Model Router     (Rust: bluey-core::router + providers)
                           ┌────────┼─────────┐
                         Fast     Vision    Reasoning        Research agent (Bun sidecar)
                           └────────┼─────────┘
                             Response Engine   (TS: src/ai/engine)
                                    │
                            Response Optimizer (TS: src/ai/optimizer)
                                    │
                                Bluey HUD
```

## Repository layout

```
src/                       React 19 + TypeScript frontend (three windows: main HUD, settings, onboarding)
  main.tsx                 entry: runs lib/tauri/bootstrap.ts, then loads the window root for the label
  app/                     styles/ (Tailwind v4 tokens), dev/ (developer overlay)
  components/ui/           design-system primitives (Radix + cva)
  features/                hud/, settings/ (tabs, incl. Privacy), onboarding/
  hooks/                   shared hooks (AI readiness, live permissions, capture protection, error presenter)
  stores/                  zustand stores mirroring backend state (never boolean soup)
  windows/                 window roots
  lib/types/               DOMAIN TYPES — mirrored 1:1 by bluey-core (camelCase on the wire)
  lib/tauri/               commands.ts (command surface), events.ts (event surface), api.ts (bluey.*),
                           bootstrap.ts, transport.ts (+ tauri-transport.ts, mock/), event-bus.ts, native HUD menu
  lib/errors/              present.ts — BlueyError kinds / codes → user copy and actions
  lib/updates/             update status copy
  lib/engine-contract.ts   UI ⇄ intelligence layer interface
  lib/auth/                auth store + gate + browser sign-in card (Rust owns the OAuth flow, ADR 0008)
  ai/                      prompt builder, request builder, streaming, generations, optimizer, engine, research
  context/                 snapshot enrichment, retrieval, fusion, token budget, intent
  transcript/              question/event classifier, speaker labelling, transcript window
  modes/                   response schemas (zod → JSON Schema), mode registry helpers, mode prompts
  sessions/                summaries, timeline, export
src-tauri/                 Rust Tauri v2 application (macOS)
  Cargo.toml               workspace root (app crate + crates/*)
  crates/bluey-core/       platform-independent domain: types, errors, state machine, events,
                           router policy, token budget, snapshot hygiene, adapters, built-in modes, shortcuts
  crates/bluey-storage/    SQLite (rusqlite): migrations, repositories, FTS5 search, documents (parse/chunk/index/retrieve), retention
  crates/bluey-protocols/  pure wire codecs (tested on any host): SSE parser, Gemini/Azure/OpenAI/Anthropic/Exa/Firecrawl
                           request+response models, subscription codecs (codex, claude_code, antigravity) with their
                           golden fingerprints/, realtime + Gemini Live (voice_live) transcription messages, sidecar
                           JSON-Lines envelopes + helper/agent mappers, panel geometry, HUD menu model, Clerk Frontend-API
                           host derivation, OAuth primitives (PKCE, URLs, callbacks)
  crates/bluey-oauth/      platform-independent OAuth runtime (host-tested): one-shot loopback listener, device-code
                           polling, token sets with single-flight refresh — used by the Clerk sign-in and the provider accounts
  crates/bluey-fingerprints/ the fingerprint capture harness behind `bun run fingerprints:*` (ADR 0009;
                           docs/PROVIDER_ACCOUNTS.md › Re-capture runbook)
  Info.plist               usage strings (microphone, speech, accessibility, screen capture) + LSUIElement, merged by the bundler (bootstrap also sets the Accessory activation policy: no Dock icon or ⌘-Tab entry)
  src/                     app crate: app/ (bootstrap, window/panel/tray setup, shutdown), commands/, state/,
                           events/ (bus + forwarder), logging/, settings/, modes/, sessions/, context/ (⌘↵ snapshot),
                           documents/, ai/ (providers, streaming, cancellation), accounts/ (provider accounts, ADR 0009),
                           sidecar/ (helper client), agent/ (research sidecar client), research/ (Exa, Firecrawl),
                           transcription/ (Gemini Live, cloud realtime, batch), capture/ audio/ accessibility/
                           (helper-backed managers), overlay/ (NSPanel), shortcuts/, permissions/, secrets/ (Keychain),
                           storage/, auth/ (browser sign-in, ADR 0008), updates/, platform/ (tray, native HUD menu)
  swift/BlueyHelper/       SwiftPM native helper (ScreenCaptureKit, Vision, AX, AVFoundation, Speech)
  capabilities/            per-window Tauri capabilities (main, settings, onboarding)
  binaries/                built sidecars (bluey-helper-*, bluey-agent-*), git-ignored
sidecars/agent/            Bun/TypeScript research sidecar: Gemini function calling (default) or the Claude Agent SDK
scripts/                   build-helper.sh, build-agent.sh, check-rust.sh, release.sh
tests/                     fixtures/ (per mode), unit/, integration/, ui/, native/, sidecar/
docs/                      this documentation + ADRs
```

## Processes

| Process | Runtime | Responsibilities | Talks to |
|---|---|---|---|
| **Bluey.app (Rust)** | Tauri v2, tokio | windows/NSPanel, tray, global shortcuts, state machine, event bus, sessions, settings, Keychain secrets, SQLite, provider HTTP + streaming, cancellation, helper/agent lifecycle, permissions | WebView (commands/events/channels), helper (stdio), agent (stdio), providers (HTTPS/WSS) |
| **WebView (TS/React)** | WKWebView | UI, stores, prompt building, context fusion, budgeting, optimisation, classification, research routing, the sign-in gate (Rust owns the OAuth flow, ADR 0008) | Rust only (typed API) |
| **bluey-helper (Swift)** | child process | screen capture, change detection, OCR, accessibility snapshots, mic + system audio, VAD, on-device speech | Rust only |
| **bluey-agent (Bun)** | per-job child | deep research with scoped tools: Gemini function calling (default) or the Claude Agent SDK | Rust only (+ Gemini/Anthropic/Exa/Firecrawl HTTPS) |

## The ⌘↵ fast path (capture + analyze)

1. Global shortcut → Rust `ShortcutManager` emits `shortcut.triggered{capture_analyze}` and
   transitions the state machine `→ capturing`.
2. HUD calls `engine.ask({trigger:"shortcut_capture", captureScreen:true, …})`.
3. Engine → `context_build_snapshot` (Rust): active app/window (helper `app.frontmost`),
   screen capture of the active display or window (helper `capture.*`, dHash change detection,
   ≤1600px JPEG) followed by OCR (Vision), the accessibility snapshot and the session context
   **in parallel**, recent transcript from the in-memory ring buffer; Rust trims everything to
   `SnapshotLimits` and applies application adapters. When the image travels with the snapshot,
   OCR is raced against a 150 ms soft deadline: a slower pass finishes into the OCR cache for
   the next ask and this snapshot carries the image alone (ADR 0010, `docs/LATENCY.md`).
   Targets: capture < 200 ms, assembly < 150 ms.
4. Engine (TS): retrieval of relevant document chunks (the mode's attached files, a small
   relevance-floored pass over session and global documents, personal instructions on every
   ask, the résumé pinned in candidate modes), context fusion into scored `ContextItem`s, token
   budget allocation, intent classification → `AIRequest` with structured-output schema for the
   mode; state `→ thinking`.
5. Rust `ai_stream`: model router picks provider/model by task/latency/vision; provider adapter
   streams SSE; chunks flow back over a `Channel`; `ai.*` events mirror to the bus.
6. Engine buffers code fences, parses structured output, optimises, saves the response and
   session event; HUD renders progressively; state `→ response_ready`.
7. Any newer request bumps the generation; the previous request of the same scope (ask, live,
   prepare) is cancelled and its late chunks dropped (ADR 0005).

## Data & privacy

* SQLite at `~/Library/Application Support/com.codewithabdul.bluey/bluey.db` (WAL). Schema in
  `crates/bluey-storage/migrations`. Screenshots are stored only when enabled; raw audio never.
* Secrets: macOS login Keychain through the native `SecKeychain*` backend (`src-tauri/src/secrets/`,
  ADR 0011), service `com.codewithabdul.bluey` (`com.codewithabdul.bluey.dev` for development builds).
* Logs: `tracing` JSON lines in `~/Library/Logs/Bluey/`, level from settings (default info);
  transcripts, screenshots, resumes and keys are never logged.

See also: `AI_ARCHITECTURE.md`, `CAPTURE_ARCHITECTURE.md`, `AUDIO_ARCHITECTURE.md`,
`MODE_SYSTEM.md`, `SECURITY.md`, `MACOS_PERMISSIONS.md`, `HELPER_PROTOCOL.md`,
`AGENT_SIDECAR_PROTOCOL.md`, `DESIGN.md`, `TESTING.md`, `DEVELOPMENT.md`, and `adr/`.
