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
  app/                     bootstrap, styles (Tailwind v4 tokens), dev overlay
  components/ui/           design-system primitives (Radix + cva)
  features/                hud/, settings/, onboarding/, sessions/, privacy/, dev/
  stores/                  zustand stores mirroring backend state (never boolean soup)
  windows/                 window roots
  lib/types/               DOMAIN TYPES — mirrored 1:1 by bluey-core (camelCase on the wire)
  lib/tauri/               commands.ts (command surface), events.ts (event surface), api.ts (bluey.*),
                           transport.ts (+ tauri-transport.ts, mock/), event-bus.ts
  lib/engine-contract.ts   UI ⇄ intelligence layer interface
  lib/auth/                Clerk integration (non-standard-browser mode)
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
  crates/bluey-protocols/  pure wire codecs (tested on any host): SSE parser, Azure/OpenAI/Anthropic/Exa/Firecrawl request+response
                           models, realtime transcription messages, sidecar JSON-Lines envelopes + helper/agent mappers,
                           panel geometry, Clerk Frontend-API host derivation
  Info.plist               usage strings (microphone, speech, accessibility, screen capture) + LSUIElement, merged by the bundler
  src/                     app crate: commands/, state/, ai/ (providers, streaming, cancellation), sidecar/ (helper client),
                           agent/ (research sidecar client), research/ (Exa, Firecrawl), transcription/ (cloud realtime),
                           capture/ audio/ accessibility/ (helper-backed managers), overlay/ (NSPanel), shortcuts/, tray/,
                           permissions/, secrets/ (Keychain), storage/, auth/, platform/
  swift/BlueyHelper/       SwiftPM native helper (ScreenCaptureKit, Vision, AX, AVFoundation, Speech)
  capabilities/            per-window Tauri capabilities (main, settings, onboarding)
  binaries/                built sidecars (bluey-helper-*, bluey-agent-*), git-ignored
sidecars/agent/            Bun/TypeScript Claude Agent SDK research sidecar
scripts/                   build-helper.sh, build-agent.sh, check-rust.sh, release.sh
tests/                     fixtures/ (per mode), unit/, integration/, ui/, native/, sidecar/
docs/                      this documentation + ADRs
```

## Processes

| Process | Runtime | Responsibilities | Talks to |
|---|---|---|---|
| **Bluey.app (Rust)** | Tauri v2, tokio | windows/NSPanel, tray, global shortcuts, state machine, event bus, sessions, settings, Keychain secrets, SQLite, provider HTTP + streaming, cancellation, helper/agent lifecycle, permissions | WebView (commands/events/channels), helper (stdio), agent (stdio), providers (HTTPS/WSS) |
| **WebView (TS/React)** | WKWebView | UI, stores, prompt building, context fusion, budgeting, optimisation, classification, research routing, Clerk auth | Rust only (typed API) |
| **bluey-helper (Swift)** | child process | screen capture, change detection, OCR, accessibility snapshots, mic + system audio, VAD, on-device speech | Rust only |
| **bluey-agent (Bun)** | per-job child | Claude Agent SDK deep research with scoped tools | Rust only (+ Anthropic/Exa/Firecrawl HTTPS) |

## The ⌘↵ fast path (capture + analyze)

1. Global shortcut → Rust `ShortcutManager` emits `shortcut.triggered{capture_analyze}` and
   transitions the state machine `→ capturing`.
2. HUD calls `engine.ask({trigger:"shortcut_capture", captureScreen:true, …})`.
3. Engine → `context_build_snapshot` (Rust): active app/window (helper `app.frontmost`),
   screen capture of the active display or window (helper `capture.*`, dHash change detection,
   ≤1600px JPEG), OCR (Vision) and accessibility snapshot **in parallel**, recent transcript from
   the in-memory ring buffer; Rust trims everything to `SnapshotLimits` and applies application
   adapters. Targets: capture < 200 ms, assembly < 150 ms.
4. Engine (TS): retrieval of relevant document chunks (session → mode → global), context fusion
   into scored `ContextItem`s, token budget allocation, intent classification → `AIRequest`
   with structured-output schema for the mode; state `→ thinking`.
5. Rust `ai_stream`: model router picks provider/model by task/latency/vision; provider adapter
   streams SSE; chunks flow back over a `Channel`; `ai.*` events mirror to the bus.
6. Engine buffers code fences, parses structured output, optimises, saves the response and
   session event; HUD renders progressively; state `→ response_ready`.
7. Any newer request bumps the generation; the previous request is cancelled and its late chunks
   dropped (ADR 0005).

## Data & privacy

* SQLite at `~/Library/Application Support/com.codewithabdul.bluey/bluey.db` (WAL). Schema in
  `crates/bluey-storage/migrations`. Screenshots are stored only when enabled; raw audio never.
* Secrets: macOS Keychain (`keyring`), service `com.codewithabdul.bluey`.
* Logs: `tracing` JSON lines in `~/Library/Logs/Bluey/`, level from settings (default info);
  transcripts, screenshots, resumes and keys are never logged.

See also: `AI_ARCHITECTURE.md`, `CAPTURE_ARCHITECTURE.md`, `AUDIO_ARCHITECTURE.md`,
`MODE_SYSTEM.md`, `SECURITY.md`, `MACOS_PERMISSIONS.md`, `HELPER_PROTOCOL.md`,
`AGENT_SIDECAR_PROTOCOL.md`, `DESIGN.md`, `TESTING.md`, `DEVELOPMENT.md`, and `adr/`.
