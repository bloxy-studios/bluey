# bluey-agent — deep-research agent sidecar

Deep, multi-step research for Bluey runs in this separate sidecar process
(`bluey-agent`), spawned per job by the Rust backend and speaking **stdio JSON
Lines** (contract: [`docs/AGENT_SIDECAR_PROTOCOL.md`](../../docs/AGENT_SIDECAR_PROTOCOL.md)).
It drives the **Claude Agent SDK** (`@anthropic-ai/claude-agent-sdk`) with a
tightly scoped, in-process MCP tool set: `exa_search`, `firecrawl_scrape`,
`document_read`.

```
Rust (Tauri v2) ── spawn per job, env-injected keys ──▶ bluey-agent (this program, Bun)
      ▲   │ stdin:  research.run / research.cancel / document.response          │
      │   ▼ stdout: research.* events, document.request                         ▼
   frontend                                            Claude Agent SDK ──▶ claude CLI subprocess
                                                        (in-process MCP server "bluey":
                                                         exa_search / firecrawl_scrape / document_read)
```

## Layout

| file | role |
|---|---|
| `src/main.ts` | stdin JSON-Lines loop, one job per process, SIGTERM/SIGINT handling, exit after the job |
| `src/protocol.ts` | wire types (copied from `src/lib/types/*`, never imported), line parser, `ProtocolWriter` |
| `src/agent.ts` | builds the MCP server, runs `query()`, maps SDK messages → protocol events |
| `src/system-prompt.ts` | research-analyst persona (citations, fact vs. inference, privacy rule) |
| `src/tools/exa.ts` | `POST https://api.exa.ai/search` client (timeouts, typed errors) |
| `src/tools/firecrawl.ts` | `POST https://api.firecrawl.dev/v2/scrape` client (markdown, truncation) |
| `src/tools/documents.ts` | `document.request`/`document.response` round-trip broker (allow-list, 10 s timeout) |
| `src/citations.ts` | URL-normalised citation dedupe + model-citation validation |
| `src/cli-path.ts` | Claude CLI resolution (`BLUEY_CLAUDE_CLI` → embedded `$bunfs` extract → SDK auto-detect) |
| `src/config.ts` | env config |
| `src/mock.ts` | `BLUEY_AGENT_MOCK=1` fake `query()` + fake search/scrape clients |
| `src/entry-darwin-{arm64,x64}.ts` | compiled entrypoints that embed the per-arch Claude CLI binary |

## Security model

- **Credentials** (`ANTHROPIC_API_KEY`, `EXA_API_KEY`, `FIRECRAWL_API_KEY`) are
  injected by Rust as environment variables from the OS keychain. They are never
  written to the protocol; error messages name the missing *variable*, never a value.
- **No filesystem / shell / built-in tools for the model.** The SDK is run with
  `tools: []` (removes every built-in tool), `allowedTools` limited to the
  requested `mcp__bluey__*` tools, `disallowedTools` re-banning Bash/Read/Write/
  Edit/WebFetch/WebSearch/Glob/Grep/Task/… (belt and braces), and
  `permissionMode: "dontAsk"` (nothing prompts; anything not pre-approved is
  denied). `cwd` is a fresh empty temp dir, `settingSources: []` (no user or
  project settings are loaded), `persistSession: false` (no transcript on disk).
- **`document_read` never touches SQLite.** It emits a `document.request` event
  and waits (10 s) for Rust's `document.response`. Ids outside the job's
  `allowedDocumentIds` are refused *without* emitting a request.
- **Citations can't be invented.** Every exa/firecrawl result observed during
  the run is recorded; the final citation list contains only URLs the tools
  actually returned. Model-chosen citations are kept (first, with their titles
  and snippets) when they match an observed URL, then the remaining observed
  sources are appended, all deduped by normalised URL.
- **The query/goal are public by design** — the system prompt forbids
  requesting, inferring, or including private user data.

## Protocol examples

Run (Rust → agent), one JSON object per line:

```jsonc
{"id":1,"method":"research.run","params":{
  "jobId":"job-1","query":"public search query","goal":"what the report must answer",
  "maxTurns":12,"tools":["exa_search","firecrawl_scrape","document_read"],
  "allowedDocumentIds":["doc-1"],"model":"claude-sonnet-5"}}
```

Agent → Rust:

```jsonc
{"id":1,"result":{"accepted":true}}
{"event":"research.started","data":{"jobId":"job-1","model":"claude-sonnet-5"}}
{"event":"research.progress","data":{"jobId":"job-1","message":"agent session started (model …, 3 tool(s))"}}
{"event":"research.toolCall","data":{"jobId":"job-1","tool":"exa_search","input":{"query":"…"}}}
{"event":"document.request","data":{"requestId":"docreq-1","documentId":"doc-1"}}
{"event":"research.textDelta","data":{"jobId":"job-1","text":"## Findings…"}}
{"event":"research.completed","data":{"jobId":"job-1","report":"…markdown…",
  "citations":[{"title":"…","url":"https://…","snippet":"…"}],
  "turns":6,"totalMs":48211,"usage":{"inputTokens":51234,"outputTokens":2101}}}
```

Cancel / document answer (Rust → agent):

```jsonc
{"id":2,"method":"research.cancel","params":{"jobId":"job-1"}}          // → {"id":2,"result":{"cancelled":true}} then research.failed {code:"cancelled",kind:"cancelled"}
{"id":3,"method":"document.response","params":{"requestId":"docreq-1","documentId":"doc-1","text":"…"}}  // fire-and-forget: no reply frame
```

Behaviour notes:

- **One job per process.** A second `research.run` gets `job_already_running`.
  The process exits (code 0) once the job completes / fails / is cancelled —
  failures are reported in-band via `research.failed`.
- **stdin EOF ≠ cancel.** A running job continues to completion; cancellation is
  `research.cancel` or SIGTERM/SIGINT. (Shell pipes close stdin immediately.)
- `research.failed.error.kind`: `"research"` for agent failures,
  `"cancelled"` for cancellation, `"configuration"` for `missing_api_key`
  (all valid `BlueyErrorKind`s).
- Failure codes: `cancelled`, `missing_api_key`, `max_turns_exceeded`,
  `budget_exceeded`, `structured_output_failed`, `agent_execution_failed`,
  `agent_empty_report`, `agent_no_result`.
- Envelope errors (`{id,error}`) use kind `"sidecar"`: `invalid_request`,
  `invalid_params`, `unknown_method`, `unknown_job`, `job_already_running`.
- Malformed lines get `{"id":null,"error":{code:"invalid_request",…}}`.

## Environment

| var | meaning | default |
|---|---|---|
| `ANTHROPIC_API_KEY` | Claude API key (required unless mock) | – |
| `EXA_API_KEY` | required when `exa_search` is requested | – |
| `FIRECRAWL_API_KEY` | required when `firecrawl_scrape` is requested | – |
| `BLUEY_RESEARCH_MODEL` (or legacy `BLUEY_MODEL_RESEARCH`) | research model | `claude-sonnet-5` |
| `BLUEY_AGENT_MAX_TURNS` | default max agent turns (request `maxTurns` overrides) | `12` |
| `BLUEY_AGENT_MOCK` | `1`/`true` → mock mode (no network/model/CLI) | off |
| `BLUEY_CLAUDE_CLI` | explicit path to the claude CLI binary (dev override) | auto |

## Build & test

```sh
# from the repo root
bun run build:agent        # scripts/build-agent.sh → src-tauri/binaries/bluey-agent-{aarch64,x86_64}-apple-darwin
bun run typecheck          # includes tsc -p sidecars/agent/tsconfig.json

# from sidecars/agent
bun install                # note: needs both darwin CLI packages for compiling — see scripts/build-agent.sh
bun run typecheck
bun run test               # vitest, tests live in tests/sidecar/ (network-free)
```

The compiled binaries embed the platform's native `claude` CLI
(`@anthropic-ai/claude-agent-sdk-darwin-{arm64,x64}/claude`) via
`import … with { type: "file" }`; at startup it is extracted from Bun's
`$bunfs` with `extractFromBunfs()` (`@anthropic-ai/claude-agent-sdk/extract`)
and passed as `pathToClaudeCodeExecutable`. That per-arch static import is why
there is one compiled entry file per target instead of compiling `main.ts`.

## Mock mode (exercise the protocol from a shell)

```sh
cd sidecars/agent

# plain run: accepted → started → progress → toolCall → textDelta → completed
printf '%s\n' '{"id":1,"method":"research.run","params":{"jobId":"job-1","query":"what is bluey","goal":"explain bluey","tools":["exa_search","firecrawl_scrape"]}}' \
  | BLUEY_AGENT_MOCK=1 bun src/main.ts

# document round-trip (request ids are deterministic: docreq-1, docreq-2, …)
{ printf '%s\n' '{"id":1,"method":"research.run","params":{"jobId":"job-2","query":"docs","goal":"docs","tools":["document_read"],"allowedDocumentIds":["doc-42"]}}';
  sleep 0.5;
  printf '%s\n' '{"id":2,"method":"document.response","params":{"requestId":"docreq-1","documentId":"doc-42","text":"hello from rust"}}';
  sleep 2; } | BLUEY_AGENT_MOCK=1 bun src/main.ts
```

Mock mode swaps in fake `query()` / Exa / Firecrawl implementations but keeps
the real protocol loop, tool handlers, citation store and document broker. A
doc-only mock run finishes with `citations: []` — the mock model cites a URL no
tool observed, and unobserved URLs are always dropped (by design).

## Known limitations

- **Binary size ~250–280 MB per arch.** The Claude CLI is a native binary
  (~250 MB) embedded inside the Bun single-file executable, and it is extracted
  to `/tmp/claude-<uid>/claude-agent-sdk-<hash>/` on first run (content-hashed:
  re-used across runs, re-extracted per SDK version).
- **Bun compile caveats.** `import … with { type: "file" }` must be statically
  analyzable, hence per-target entry files; `bun install` must have the darwin
  platform optional deps present (host-filtered by default — the build script
  uses `bun install --os darwin --cpu '*'`).
- **Model availability.** The default `claude-sonnet-5` must be available to
  the key; override per-request (`model`) or via `BLUEY_RESEARCH_MODEL`.
- The SDK spawns the CLI as a subprocess; first token latency includes that
  startup (~1 s). `usage.inputTokens` sums fresh + cache-created + cache-read
  input tokens of the main agent loop.
- `research.progress` for tool activity is emitted from the tool handlers
  (results/errors), not from raw SDK `tool_result` frames; `research.toolCall`
  reports bare tool names (`exa_search`, not `mcp__bluey__exa_search`).
