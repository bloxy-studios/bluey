# bluey-agent — deep-research agent sidecar

Deep, multi-step research for Bluey runs in this separate sidecar process
(`bluey-agent`), spawned per job by the Rust backend and speaking **stdio JSON
Lines** (contract: [`docs/AGENT_SIDECAR_PROTOCOL.md`](../../docs/AGENT_SIDECAR_PROTOCOL.md)).
It drives one of two model backends with the same tightly scoped tool set
(`exa_search`, `firecrawl_scrape`, `document_read`):

- **Gemini (default, `RESEARCH_BACKEND=gemini`)** — a function-calling loop over
  `@google/genai` (`generateContentStream`, Google AI Studio key). Pure JS, no
  subprocess, a few-MB binary.
- **Claude (`RESEARCH_BACKEND=claude`)** — the **Claude Agent SDK**
  (`@anthropic-ai/claude-agent-sdk`) driving the Claude Code CLI, Anthropic
  direct or through Microsoft Foundry.

```
Rust (Tauri v2) ── spawn per job, env-injected keys ──▶ bluey-agent (this program, Bun)
      ▲   │ stdin:  research.run / research.cancel / document.response          │
      │   ▼ stdout: research.* events, document.request                         ▼
   frontend                      RESEARCH_BACKEND=gemini ──▶ @google/genai generateContentStream (function calling)
                                 RESEARCH_BACKEND=claude ──▶ Claude Agent SDK ──▶ claude CLI subprocess
                                                              (in-process MCP server "bluey")
                                 tools for both: exa_search / firecrawl_scrape / document_read
```

## Layout

| file                                   | role                                                                                          |
| -------------------------------------- | --------------------------------------------------------------------------------------------- |
| `src/main.ts`                          | stdin JSON-Lines loop, one job per process, SIGTERM/SIGINT handling, exit after the job       |
| `src/protocol.ts`                      | wire types (copied from `src/lib/types/*`, never imported), line parser, `ProtocolWriter`     |
| `src/agent.ts`                         | tool handlers, backend switch, Claude `query()` loop, SDK message → protocol event mapping    |
| `src/gemini.ts`                        | Gemini function-calling loop + final schema-constrained report turn, API error mapping        |
| `src/tool-specs.ts`                    | tool names/descriptions/zod shapes shared by both backends (→ MCP tools / function declarations) |
| `src/system-prompt.ts`                 | research-analyst persona (citations, fact vs. inference, privacy rule)                        |
| `src/tools/exa.ts`                     | `POST https://api.exa.ai/search` client (timeouts, typed errors)                              |
| `src/tools/firecrawl.ts`               | `POST https://api.firecrawl.dev/v2/scrape` client (markdown, truncation)                      |
| `src/tools/documents.ts`               | `document.request`/`document.response` round-trip broker (allow-list, 10 s timeout)           |
| `src/citations.ts`                     | URL-normalised citation dedupe + model-citation validation                                    |
| `src/cli-path.ts`                      | Claude CLI resolution (`BLUEY_CLAUDE_CLI` → embedded `$bunfs` extract → SDK auto-detect)      |
| `src/config.ts`                        | env config (`RESEARCH_BACKEND`, keys, model, turns)                                           |
| `src/mock.ts`                          | `BLUEY_AGENT_MOCK=1` fake Gemini/Claude models + fake search/scrape clients                   |
| `src/entry-darwin-{arm64,x64}-lite.ts` | compiled entrypoints of the default **lite** build (no embedded Claude CLI)                   |
| `src/entry-darwin-{arm64,x64}.ts`      | compiled entrypoints of the **full** build (embed the per-arch Claude CLI binary)             |

## Security model

- **Credentials** (`GEMINI_API_KEY` — alias `GOOGLE_API_KEY` — for Gemini;
  `ANTHROPIC_API_KEY` or the `ANTHROPIC_FOUNDRY_*` set for Claude; `EXA_API_KEY`,
  `FIRECRAWL_API_KEY`) are injected by Rust as environment variables from the OS
  keychain. They are never written to the protocol; error messages name the
  missing _variable_, never a value. API error bodies are never forwarded either
  (they can echo prompt text) — only the HTTP status is mapped onto a code.
- **The Claude Code subprocess only sees the routing we decided.** `buildSubprocessEnv`
  clears every provider variable (`ANTHROPIC_*`, `CLAUDE_CODE_USE_FOUNDRY`,
  `GEMINI_API_KEY`, `GOOGLE_API_KEY`) from the inherited environment and re-adds
  exactly one Claude configuration (Anthropic direct _or_ Microsoft Foundry), so
  a stray `ANTHROPIC_BASE_URL` can never redirect the agent and the Gemini key
  never reaches the CLI.
- **No filesystem / shell / built-in tools for the model.** Gemini only ever
  receives the three function declarations. The Claude SDK is run with
  `tools: []` (removes every built-in tool), `allowedTools` limited to the
  requested `mcp__bluey__*` tools, `disallowedTools` re-banning Bash/Read/Write/
  Edit/WebFetch/WebSearch/Glob/Grep/Task/… (belt and braces), and
  `permissionMode: "dontAsk"` (nothing prompts; anything not pre-approved is
  denied). `cwd` is a fresh empty temp dir, `settingSources: []` (no user or
  project settings are loaded), `persistSession: false` (no transcript on disk).
- **`document_read` never touches SQLite.** It emits a `document.request` event
  and waits (10 s) for Rust's `document.response`. Ids outside the job's
  `allowedDocumentIds` are refused _without_ emitting a request.
- **Citations can't be invented.** Every exa/firecrawl result observed during
  the run is recorded; the final citation list contains only URLs the tools
  actually returned. Model-chosen citations are kept (first, with their titles
  and snippets) when they match an observed URL, then the remaining observed
  sources are appended, all deduped by normalised URL.
- **The query/goal are public by design** — the system prompt forbids
  requesting, inferring, or including private user data.

## Gemini backend (default)

`src/gemini.ts` — verified against ai.google.dev on 2026-09-08 (see
`docs/reference/gemini-api-sept-2026.md`):

1. Every turn calls `ai.models.generateContentStream({ model, contents, config })`
   with `systemInstruction`, `tools: [{ functionDeclarations }]` (derived from the
   zod shapes in `tool-specs.ts` via `parametersJsonSchema`),
   `thinkingConfig: { thinkingLevel: LOW }`, the job's `abortSignal` and
   `httpOptions: { timeout: 60_000, retryOptions: { attempts: 3 } }`. Text parts
   stream out as `research.textDelta` (thought summaries are skipped).
2. The model turn is pushed back **unchanged** (it carries `thoughtSignature`s),
   each `functionCall` is executed through the shared handlers
   (`research.toolCall` + `research.progress`), and one `user` turn with all
   `functionResponse` parts — `id` echoed from the call — is appended.
3. When a turn has no function calls (or the turn budget leaves exactly one
   turn), a final turn without tools asks for the report with
   `responseMimeType: "application/json"` + `responseJsonSchema`
   (`{ report, citations }`). Its citations are validated by `CitationStore`.
4. `turns` counts model round-trips (tool turns + the report turn);
   `usage` sums `promptTokenCount` and `candidatesTokenCount + thoughtsTokenCount`.

Never sent on Gemini 3.x: `temperature`/`topP`/`topK`, `candidateCount`,
`thinkingBudget`. The default model is `gemini-3.8-flash` (`BLUEY_RESEARCH_MODEL`
overrides; `gemini-3.5-flash-lite` is the cheap alternative).

## Protocol examples

Run (Rust → agent), one JSON object per line:

```jsonc
{
  "id": 1,
  "method": "research.run",
  "params": {
    "jobId": "job-1",
    "query": "public search query",
    "goal": "what the report must answer",
    "maxTurns": 12,
    "tools": ["exa_search", "firecrawl_scrape", "document_read"],
    "allowedDocumentIds": ["doc-1"],
    "model": "gemini-3.8-flash",
  },
}
```

Agent → Rust:

```jsonc
{"id":1,"result":{"accepted":true}}
{"event":"research.started","data":{"jobId":"job-1","model":"gemini-3.8-flash"}}
{"event":"research.progress","data":{"jobId":"job-1","message":"agent session started (model …, 3 tool(s))"}}
{"event":"research.toolCall","data":{"jobId":"job-1","tool":"exa_search","input":{"query":"…"}}}
{"event":"document.request","data":{"requestId":"docreq-1","documentId":"doc-1"}}
{"event":"research.textDelta","data":{"jobId":"job-1","text":"## Findings…"}}
{"event":"research.progress","data":{"jobId":"job-1","message":"writing the report"}}
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
  `"cancelled"` for cancellation, `"configuration"` for `missing_api_key` /
  `invalid_api_key` / `invalid_configuration` (all valid `BlueyErrorKind`s).
- Failure codes: `cancelled`, `missing_api_key`, `invalid_api_key` (Gemini: key
  rejected, HTTP 400/401/403), `invalid_configuration`, `max_turns_exceeded`,
  `rate_limited` (Gemini HTTP 429), `blocked` (Gemini refused the prompt or
  answer), `budget_exceeded`, `structured_output_failed`,
  `agent_execution_failed`, `agent_empty_report`, `agent_no_result`.
- Envelope errors (`{id,error}`) use kind `"sidecar"`: `invalid_request`,
  `invalid_params`, `unknown_method`, `unknown_job`, `job_already_running`.
- Malformed lines get `{"id":null,"error":{code:"invalid_request",…}}`.

## Environment

| var                                                       | meaning                                                                                  | default                                             |
| --------------------------------------------------------- | ---------------------------------------------------------------------------------------- | --------------------------------------------------- |
| `RESEARCH_BACKEND`                                        | `gemini` \| `claude` — which model loop runs the job                                     | `gemini`                                            |
| `GEMINI_API_KEY` (alias `GOOGLE_API_KEY`)                 | Google AI Studio key — required by the Gemini backend                                    | –                                                   |
| `ANTHROPIC_API_KEY`                                       | Claude API key — Anthropic direct (Claude backend, unless Foundry or mock)               | –                                                   |
| `EXA_API_KEY`                                             | required when `exa_search` is requested                                                  | –                                                   |
| `FIRECRAWL_API_KEY`                                       | required when `firecrawl_scrape` is requested                                            | –                                                   |
| `BLUEY_RESEARCH_MODEL` (or legacy `BLUEY_MODEL_RESEARCH`) | research model (a Foundry _deployment name_ when Foundry is on)                          | `gemini-3.8-flash` (Gemini) / `claude-sonnet-5` (Claude) |
| `BLUEY_AGENT_MAX_TURNS`                                   | default max agent turns (request `maxTurns` overrides)                                   | `12`                                                |
| `BLUEY_AGENT_MOCK`                                        | `1`/`true` → mock mode (no network/model/CLI) for either backend                         | off                                                 |
| `BLUEY_CLAUDE_CLI`                                        | explicit path to the claude CLI binary (dev override; needed by the lite build for Claude) | auto                                              |

### Claude through Microsoft Foundry

The Agent SDK drives Claude Code, which supports Foundry natively through
environment variables (`src/config.ts` → `buildSubprocessEnv` in `src/agent.ts`).
Everything below is handed to the Claude Code subprocess; the endpoint becomes
`https://{resource}.services.ai.azure.com/anthropic` and every `model` is a Foundry
**deployment name** (defaults to the model id, e.g. `claude-opus-5`).

| var                                                                 | meaning                                                                                                             | default                                        |
| ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| `CLAUDE_CODE_USE_FOUNDRY`                                           | `1`/`true` → route through Microsoft Foundry                                                                        | off                                            |
| `ANTHROPIC_FOUNDRY_RESOURCE`                                        | Foundry resource name                                                                                               | derived from `AZURE_FOUNDRY_ENDPOINT` hostname |
| `ANTHROPIC_FOUNDRY_BASE_URL`                                        | full base URL (only used when no resource name is available; Claude Code rejects both)                              | –                                              |
| `ANTHROPIC_FOUNDRY_API_KEY`                                         | Foundry resource key                                                                                                | `AZURE_FOUNDRY_API_KEY`                        |
| `ANTHROPIC_FOUNDRY_AUTH_TOKEN`                                      | Entra ID bearer token (takes precedence over the key)                                                               | –                                              |
| `ANTHROPIC_DEFAULT_OPUS_MODEL` / `…_SONNET_MODEL` / `…_HAIKU_MODEL` | pinned deployment names for the `opus` / `sonnet` / `haiku` aliases (Foundry has no startup model check — pin them) | Claude Code built-ins                          |

Failure modes before the CLI is spawned: `invalid_configuration` (Foundry on but no
resource / base URL) and `missing_api_key` naming `ANTHROPIC_FOUNDRY_API_KEY`.
Current Claude ids on Foundry: `claude-opus-5`, `claude-sonnet-5`, `claude-haiku-4-5`
(Hosted on Azure or on Anthropic infrastructure); `claude-fable-5` / `-5-1` are preview,
Anthropic-hosted only.

## Build & test

```sh
# from the repo root
bun run build:agent                              # lite variant (default): src-tauri/binaries/bluey-agent-{aarch64,x86_64}-apple-darwin
BLUEY_AGENT_VARIANT=full bun run build:agent     # embed the Claude CLI (~250 MB per arch)
bun run typecheck                                # includes tsc -p sidecars/agent/tsconfig.json

# from sidecars/agent
bun install
bun run typecheck
bun run test               # vitest, tests live in tests/sidecar/ (network-free)
```

Variants (`scripts/build-agent.sh`, `BLUEY_AGENT_VARIANT=lite|full`; the default
becomes `full` when `RESEARCH_BACKEND=claude` is exported):

- **lite** compiles `src/entry-darwin-*-lite.ts`: only `@google/genai` and the
  tool clients are bundled. The Claude backend still runs from a lite binary
  when `BLUEY_CLAUDE_CLI` points at an installed Claude Code CLI.
- **full** compiles `src/entry-darwin-*.ts`, which embed the platform's native
  `claude` CLI (`@anthropic-ai/claude-agent-sdk-darwin-{arm64,x64}/claude`) via
  `import … with { type: "file" }`; at startup it is extracted from Bun's
  `$bunfs` with `extractFromBunfs()` and passed as `pathToClaudeCodeExecutable`.
  That per-arch static import is why there is one compiled entry file per target.

`@google/genai` is pinned to `^2.21.0` (2.x; v3 requires Node 22 and drops
automatic function calling from `generateContent`). It is declared in this
package **and** in the repo root `package.json` because `tests/sidecar/*` import
the sidecar sources through the root vitest config.

### Bun compatibility of `@google/genai`

Undocumented by Google. Verified in this repo: the SDK loads and the Gemini
mock job runs end-to-end under `bun src/main.ts` (Bun 1.x). Live
`generateContentStream` traffic from the compiled lite binary must still be
confirmed against a real key on macOS (`RESEARCH_BACKEND=gemini
GEMINI_API_KEY=… bun src/main.ts`); if the SDK's HTTP layer misbehaves under
Bun, the fallback plan is a ~200-line `fetch` + SSE client behind the same
`GenerateFn` interface (`src/gemini.ts` isolates the SDK behind it).

## Mock mode (exercise the protocol from a shell)

```sh
cd sidecars/agent

# plain run (Gemini loop): accepted → started → progress → toolCall → textDelta → "writing the report" → completed
printf '%s\n' '{"id":1,"method":"research.run","params":{"jobId":"job-1","query":"what is bluey","goal":"explain bluey","tools":["exa_search","firecrawl_scrape"]}}' \
  | BLUEY_AGENT_MOCK=1 bun src/main.ts

# same job through the Claude mock
printf '%s\n' '{"id":1,"method":"research.run","params":{"jobId":"job-1","query":"what is bluey","goal":"explain bluey","tools":["exa_search","firecrawl_scrape"]}}' \
  | BLUEY_AGENT_MOCK=1 RESEARCH_BACKEND=claude bun src/main.ts

# document round-trip (request ids are deterministic: docreq-1, docreq-2, …)
{ printf '%s\n' '{"id":1,"method":"research.run","params":{"jobId":"job-2","query":"docs","goal":"docs","tools":["document_read"],"allowedDocumentIds":["doc-42"]}}';
  sleep 0.5;
  printf '%s\n' '{"id":2,"method":"document.response","params":{"requestId":"docreq-1","documentId":"doc-42","text":"hello from rust"}}';
  sleep 2; } | BLUEY_AGENT_MOCK=1 bun src/main.ts
```

Mock mode swaps in a fake model (`createMockGeminiGenerate` scripts one function
call per turn and lets the real loop execute the handlers; `createMockQueryFn`
scripts Claude SDK messages) plus fake Exa / Firecrawl clients, but keeps the
real protocol loop, tool handlers, citation store and document broker. A
doc-only mock run finishes with `citations: []` — the mock model cites a URL no
tool observed, and unobserved URLs are always dropped (by design).

## Known limitations

- **Binary size.** Lite: a few MB. Full: ~250–280 MB per arch — the Claude CLI
  is a native binary embedded inside the Bun single-file executable, extracted
  to `/tmp/claude-<uid>/claude-agent-sdk-<hash>/` on first run.
- **Bun compile caveats (full).** `import … with { type: "file" }` must be
  statically analyzable, hence per-target entry files; `bun install` must have
  the darwin platform optional deps present (host-filtered by default — the
  build script uses `bun install --os darwin --cpu '*'`).
- **Model availability.** Gemini: the default `gemini-3.8-flash` is on the free
  tier but free-tier daily quotas are tiny (429 → `rate_limited`); link billing
  in AI Studio for real use. Claude: `claude-sonnet-5` must be available to the
  key (on Foundry: deployed under that name). Override per request (`model`) or
  via `BLUEY_RESEARCH_MODEL`.
- **Gemini structured output.** The tool loop runs in text mode and the report
  is produced by one extra schema-constrained turn (the robust pattern on 3.x);
  `turns` therefore includes that final turn.
- **Claude only via the SDK.** The Agent SDK talks the Anthropic Messages API
  (direct, Bedrock, Vertex or Foundry); it cannot drive GPT or Gemini models.
  The SDK spawns the CLI as a subprocess; first token latency includes that
  startup (~1 s). `usage.inputTokens` sums fresh + cache-created + cache-read
  input tokens of the main agent loop.
- `research.progress` for tool activity is emitted from the tool handlers
  (results/errors), not from raw model frames; `research.toolCall` reports bare
  tool names (`exa_search`, not `mcp__bluey__exa_search`).
