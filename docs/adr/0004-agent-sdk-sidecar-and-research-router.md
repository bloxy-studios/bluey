# ADR 0004 — Claude Agent SDK in a Bun sidecar behind a Research Router

**Status:** accepted · **Date:** 2026-09-07

## Context
The spec wants agentic deep research through the Claude Agent SDK, but never an agent loop for
normal responses, and never unrestricted shell/filesystem/network access for the live assistant.
The Agent SDK is a Node/Bun library that drives a native Claude CLI binary; it cannot run inside
the WebView.

## Decision
* A **Research Router** (TS, `src/ai/research/router.ts`) chooses the cheapest path:
  `none` → `search` (Exa via Rust) → `search_scrape` (Exa + Firecrawl via Rust) → `deep_agent`.
* `deep_agent` spawns `bluey-agent`, a Bun single-file executable containing the Agent SDK. Rust
  injects `ANTHROPIC_API_KEY`, `EXA_API_KEY`, `FIRECRAWL_API_KEY` into its environment from the
  Keychain. The agent runs `query()` with `tools: []` (no built-ins), an in-process MCP server
  exposing only `exa_search`, `firecrawl_scrape`, `document_read`, and an allow-list for exactly
  those tools. Document reads are served by Rust from SQLite for an explicit id allow-list.
* Public/private separation: the router builds a **public query** (no names, emails, resume
  facts) for web tools; private context is combined locally after results return.

## Consequences
* Normal answers keep sub-second first-token latency (single provider call from Rust).
* The sidecar is per job, so a crash or runaway loop never affects the app; `maxTurns` and an
  `AbortController` bound cost.
* Requires the platform-specific SDK binary at build time (`scripts/build-agent.sh`).

## Addendum — 2026-09-28 (deep-refinement audit)
* The router lives in `src/ai/research.ts` (`decideResearch`, `buildPublicQuery`,
  `runResearch`); the engine calls it from `maybeResearch` in `src/ai/engine.ts`. There is no
  `src/ai/research/router.ts`.
* The sidecar's default backend is **Gemini** function calling (ADR 0007); the Claude Agent SDK
  is the opt-in `claude` backend and needs the full build (the lite build has no Claude Code CLI).
  `research_available` reports the backends the installed build can run (`agent.info`), and
  deep research is offered only with an Exa key; the job requests only the tools that have keys.
* Rust injects exactly one backend's model credential plus the Exa/Firecrawl keys. The Claude
  Code subprocess never receives the tool keys and runs with telemetry, error reporting and
  non-essential traffic disabled.
* Public query: the user's typed ask only. `buildPublicQuery` strips emails, phone numbers,
  handles and the signed-in user's names, passed in explicitly by the engine. It is a
  best-effort scrub, not a guarantee; private documents are never added to the query, and
  words the ask shares with them (a skill, a past employer) are kept as the search topic.
* Bounds: a cancelled or superseded ask cancels its job; the agent gets a `deadlineMs` 15 s inside
  the 90 s ask timeout and a turn budget, and when either runs out it reports from the evidence
  gathered instead of failing. The search paths have a 10 s overall deadline.
