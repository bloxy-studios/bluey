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
* **Self-updating** — signed in-app updates on a **Latest** (stable) or **Nightly** channel,
  automatic by default: Bluey downloads and installs in the background and asks to relaunch
  (`docs/UPDATES.md`).

Bluey is a personal tool: **no billing, subscriptions or teams of its own** — you bring your own
AI provider keys or, where supported, your own AI subscriptions. Sign-in uses Clerk, in your browser.

## Stack
Tauri v2 · Rust (tokio, rusqlite, reqwest) · Swift native helper (ScreenCaptureKit, Vision,
Accessibility, AVFoundation, Speech) · React 19 + TypeScript + Tailwind v4 + Zustand · Clerk ·
Google Gemini (default — one AI Studio key for chat, vision, transcription, embeddings and
research) with Microsoft Foundry / Azure OpenAI, Anthropic and OpenAI-compatible providers as
alternates · research sidecar (Bun; Gemini function calling, or the Claude Agent SDK) · Exa ·
Firecrawl · SQLite (FTS5) · macOS Keychain.

## Requirements
macOS 14+ (Apple Silicon or Intel). To build: Xcode Command Line Tools, Bun ≥ 1.2, Rust stable.
The macOS release pipeline specifically pins **Bun 1.4.2** and requires Python 3.9+.

## Quick start
```bash
bun install
cp .env.example .env          # or .env.local — VITE_CLERK_PUBLISHABLE_KEY + BLUEY_CLERK_OAUTH_CLIENT_ID + GEMINI_API_KEY (the key is imported into the Keychain on first run)
bun run build:helpers         # Swift helper + research agent sidecars
bun run tauri:dev
```
Then press **⌘ \\** to toggle Bluey, **⌘ ↵** to ask about your screen, **⌘ ⇧ L** to start listening.

| Command | Purpose |
|---|---|
| `bun run dev` | UI in a browser with the mock backend (`?window=main|settings|onboarding&dev=1`) |
| `bun run typecheck` · `bun run lint` · `bun run test` | TypeScript checks and tests |
| `bun run test:rust` · `bun run check:rust [--darwin]` | Rust tests / type-check for macOS |
| `bun run tauri:build` · `scripts/release.sh` | local production/build-only bundles; [gated release publishing](docs/RELEASING.md) |

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
[Agent sidecar protocol](docs/AGENT_SIDECAR_PROTOCOL.md) ·
[Provider accounts](docs/PROVIDER_ACCOUNTS.md) · [Latency](docs/LATENCY.md) · [ADRs](docs/adr/)

## Platform limitations (honest list)
* Content protection hides the panel from most capture APIs but macOS does not guarantee it for
  every capture path; Bluey never attempts to defeat monitoring software.
* System audio capture requires Screen Recording permission (ScreenCaptureKit).
* Speaker identification is derived from audio channel (you vs. others) — not true diarization.
* Sign-in runs in the system browser (Clerk as OAuth/OIDC provider, PKCE, `bluey://` deep link
  back into the app — ADR 0008); nothing from Clerk runs inside the WebView.
* Signing in with a paid AI subscription (ChatGPT via Codex OAuth, Claude Pro/Max via claude.ai
  OAuth and Google AI Pro/Ultra via Antigravity OAuth; ADR 0009) is
  **unofficial and experimental**: Bluey speaks the vendors'
  own client protocols, the vendors may change or stop them without notice, and when a provider
  stops recognising Bluey it stops and falls back to your API key rather than spend paid extra
  usage. Status per provider: `docs/PROVIDER_ACCOUNTS.md`.
* Notifications permission is only available in a signed, bundled app.
