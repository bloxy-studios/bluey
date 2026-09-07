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
