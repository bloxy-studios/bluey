# Bluey

**Bluey is a real-time AI desktop copilot for macOS.** It lives in a floating panel above your
other apps and — only with permissions you explicitly grant — understands what is on your screen
and what is being said, combines that with your own context (resume, job description, notes) and
the mode you're in, and prepares concise, useful help before you even ask.

> Bluey should never feel like "I opened an AI chatbot." It should feel like
> "Bluey already understands what's happening."

## What it does
* **Screen understanding** — on-demand capture (⌘↵) of a display, window or region with
  ScreenCaptureKit, Vision OCR and Accessibility semantics; optional low-frequency smart
  observation with change detection.
* **Live transcription** — microphone and system audio (separate channels), on-device Apple
  Speech by default or a cloud realtime provider, speaker labels with honest confidence.
* **Question & event detection** — questions, coding problems, objections, decisions, action
  items… detected live and prepared silently; ⌘⇧↵ shows the prepared response instantly.
* **Context fusion** — screen + focused UI + transcript + your documents + mode + session
  memory, scored, budgeted and sent to the right model (fast / vision / reasoning / research).
* **Modes** — General, Interview, Behavioral Interview, Coding Interview, System Design, Case
  Interview, Sales, Recruiting, Team Meeting, Lecture — plus your own custom modes. Modes are
  data: instructions, output schema, latency preference, context requirements, attached files.
* **Structured responses** — natural interview answers, exact code with copy/expand, system
  design sections with Mermaid diagrams, case frameworks, sales objection handling, meeting
  decisions & action items, lecture notes and study guides.
* **Sessions** — timeline, history, search (FTS), notes, post-session summaries, export.
* **Research** — Exa / Firecrawl for current information and a Claude Agent SDK sidecar for
  deep, multi-step research with strictly scoped tools.
* **Privacy by design** — nothing captured silently; minimal retention defaults; raw audio never
  stored; Privacy display mode (platform content protection); one-click disable; real deletion.
* **Native macOS feel** — NSPanel HUD across Spaces and fullscreen apps, menu bar item,
  configurable global shortcuts, Retina/multi-display aware, dark/light, reduced motion.

Bluey is a personal tool: **no billing, subscriptions or teams.** Sign-in uses Clerk.

## Stack
Tauri v2 · Rust (tokio, rusqlite, reqwest) · Swift native helper (ScreenCaptureKit, Vision,
Accessibility, AVFoundation, Speech) · React 19 + TypeScript + Tailwind v4 + Zustand · Clerk ·
Microsoft Foundry / Azure OpenAI, Anthropic and OpenAI-compatible providers · Claude Agent SDK
(Bun sidecar) · Exa · Firecrawl · SQLite (FTS5) · macOS Keychain.

## Requirements
macOS 14+ (Apple Silicon or Intel). To build: Xcode Command Line Tools, Bun ≥ 1.2, Rust stable.

## Quick start
```bash
bun install
cp .env.example .env          # add VITE_CLERK_PUBLISHABLE_KEY (+ provider keys, or add them in Settings)
bun run build:helpers         # Swift helper + research agent sidecars
bun run tauri:dev
```
Then press **⌘ \\** to toggle Bluey, **⌘ ↵** to ask about your screen, **⌘ ⇧ L** to start listening.

| Command | Purpose |
|---|---|
| `bun run dev` | UI in a browser with the mock backend (`?window=main|settings|onboarding&dev=1`) |
| `bun run typecheck` · `bun run lint` · `bun run test` | TypeScript checks and tests |
| `bun run test:rust` · `bun run check:rust [--darwin]` | Rust tests / type-check for macOS |
| `bun run tauri:build` · `scripts/release.sh` | production build, signing and notarization |

## Default shortcuts
⌘ \\ toggle · ⌘ ↵ capture + analyze · ⌘ ⇧ ↵ generate response · ⌘ ⇧ L toggle listening ·
⌘ R new chat · ⌘ , settings · ⌘ ↑↓←→ move panel · ⌘ ⇧ ↑↓ scroll response. All remappable, with
conflict detection.

## Documentation
[Architecture](docs/ARCHITECTURE.md) · [Development](docs/DEVELOPMENT.md) ·
[macOS permissions](docs/MACOS_PERMISSIONS.md) · [AI architecture](docs/AI_ARCHITECTURE.md) ·
[Capture](docs/CAPTURE_ARCHITECTURE.md) · [Audio](docs/AUDIO_ARCHITECTURE.md) ·
[Mode system](docs/MODE_SYSTEM.md) · [Security](docs/SECURITY.md) · [Testing](docs/TESTING.md) ·
[Design](docs/DESIGN.md) · [Helper protocol](docs/HELPER_PROTOCOL.md) ·
[Agent sidecar protocol](docs/AGENT_SIDECAR_PROTOCOL.md) · [ADRs](docs/adr/)

## Platform limitations (honest list)
* Content protection hides the panel from most capture APIs but macOS does not guarantee it for
  every capture path; Bluey never attempts to defeat monitoring software.
* System audio capture requires Screen Recording permission (ScreenCaptureKit).
* Speaker identification is derived from audio channel (you vs. others) — not true diarization.
* Clerk does not officially support Tauri; Bluey uses Clerk's native (non-standard-browser) mode
  with documented fallbacks (see ADR 0003).
* Notifications permission is only available in a signed, bundled app.
