# Bluey audit — research sidecar + frontend completeness

Repository: `/agent/workspace/bluey` (Tauri v2 macOS copilot; Rust backend, Swift helper, React 19 + TS + Tailwind v4 + Zustand, Bun tooling). Read-only audit performed 2026-09-08. Line numbers refer to the files as they exist in the repo today.

> **Headline finding that reframes both parts:** the Rust app crate is a shell. `src-tauri/src/lib.rs` (14 lines) is the untouched Tauri template (`greet` command only, no `mod` declarations). The modules that do exist under `src-tauri/src/` (`ai/`, `sidecar/`, `settings/`, `secrets/`, `sessions/`, `modes/`, `state/`, `storage/`, `events/`, `logging/`, `capture/`, `accessibility/`) are never compiled into the crate, and `src-tauri/src/state/mod.rs:166-189` (`AppCore`) references modules that do not exist at all: `crate::agent::AgentManager`, `crate::research::ResearchManager`, `crate::audio`, `crate::auth`, `crate::overlay`, `crate::shortcuts`, `crate::permissions`, `crate::documents`, `crate::platform`, `crate::app`. There is **no `commands/` directory** and no `generate_handler!` list beyond `greet`. Consequently: nothing in Rust spawns `bluey-agent`, no Tauri command in `src/lib/tauri/commands.ts` is implemented natively, and `tests/integration/command-surface.test.ts` must currently fail (Rust registers `greet`; TS declares 120 commands). Everything the UI does today runs only against `MockTransport`.

---

## PART A — Research sidecar (`sidecars/agent/**`)

### A1. How deep research works today

#### A1.1 Process model & binary

| Item | Value | Source |
|---|---|---|
| Binary name | `bluey-agent` → `src-tauri/binaries/bluey-agent-{aarch64,x86_64}-apple-darwin` | `scripts/build-agent.sh:4-6,87-99`; declared in `src-tauri/tauri.conf.json:83` `"externalBin": ["binaries/bluey-helper", "binaries/bluey-agent"]` |
| Lifecycle | **One research job per process**; process exits `0` after the terminal event (completed/failed/cancelled). stdin EOF does *not* cancel a running job. SIGTERM/SIGINT → `job.cancel("termination signal")` with a 5 s safety-net exit. | `sidecars/agent/src/main.ts:72-94,148-170` |
| Args | None. Everything is env + stdin. | `main.ts:41-46` |
| Rust spawner | **Does not exist.** Intended pattern is `src-tauri/src/sidecar/mod.rs` (`HelperClient`: `app.shell().sidecar(HELPER_BIN).spawn()`, JSON-Lines reader over `CommandEvent::Stdout`, per-method timeouts `timeout_for()`, crash restart). `bluey-protocols::agent` ships the pure mappers for an `AgentManager` that was never written. | `src-tauri/src/sidecar/mod.rs:98-146,148-201,305-346`; `src-tauri/crates/bluey-protocols/src/agent.rs:23-48,52-184`; `src-tauri/src/state/mod.rs:183-184` |

#### A1.2 stdin/stdout JSON-Lines protocol (`docs/AGENT_SIDECAR_PROTOCOL.md`, `sidecars/agent/src/protocol.ts`)

Envelope is shared with the Swift helper (`bluey-protocols::jsonl`, `src-tauri/crates/bluey-protocols/src/jsonl.rs:86-123`): request `{id, method, params}`; reply `{id, result}` / `{id, error:{code,message,kind,details?}}`; event `{event, data}`. Rust's `parse_line` only accepts **string** ids (`object.get("id").and_then(Value::as_str)`, `jsonl.rs:108`), while the sidecar accepts string or number (`protocol.ts:152-157`) — fine as long as Rust always sends strings (HelperClient uses `r-<n>`).

Methods (Rust → agent), `main.ts:131-145`:

| method | params (zod) | reply |
|---|---|---|
| `research.run` | `deepResearchRequestSchema` (`protocol.ts:48-57`): `jobId`, `sessionId?`, `query`, `goal`, `maxTurns?` (int>0), `tools: ("exa_search"\|"firecrawl_scrape"\|"document_read")[]` min 1, `allowedDocumentIds?`, `model?` | `{accepted:true}` immediately; second run → `job_already_running` |
| `research.cancel` | `{jobId}` | `{cancelled:true}` then `research.failed {code:"cancelled", kind:"cancelled"}`; unknown → `unknown_job` |
| `document.response` | `{requestId, documentId, text?, error?}` | none on success (fire-and-forget); `invalid_params` error frame only if malformed |

Envelope errors use `kind:"sidecar"`: `invalid_request` (id `null` for bad JSON), `invalid_params`, `unknown_method`, `unknown_job`, `job_already_running`.

Events (agent → Rust), `protocol.ts:110-118`, mapped in `bluey-protocols/src/agent.rs:52-184` onto `bluey_core::types::DeepResearchEvent` (`crates/bluey-core/src/types/ai.rs:376-401`) and re-emitted to the WebView as `research.event` (`src/lib/tauri/events.ts:77`):

| event | data | Rust mapping note |
|---|---|---|
| `research.started` | `{jobId, model}` | `model` is **dropped** (`Started{job_id}` only) |
| `research.progress` | `{jobId, message}` | |
| `research.toolCall` | `{jobId, tool, input}` — bare tool names | `DeepResearchEvent::ToolCall` (TS tag `tool_call`) |
| `research.textDelta` | `{jobId, text}` | TS tag `text_delta` |
| `research.completed` | `{jobId, report(markdown), citations:[{title,url,snippet?}], turns, totalMs, usage:{inputTokens,outputTokens}}` | citations get ids `cit_1..n`; **`usage` is dropped** by Rust |
| `research.failed` | `{jobId, error:{code,message,kind}}` | `WireError::into_bluey` prefixes code with kind (`research.max_turns_exceeded`, `configuration.missing_api_key`, `cancelled.cancelled`) |
| `document.request` | `{requestId, documentId}` | `AgentEvent::DocumentRequest` — Rust must answer via `document_response_params` |

Failure codes emitted by the job (`agent.ts:260-285,436-478,647-662`): `cancelled` (kind `cancelled`), `missing_api_key` / `invalid_configuration` (kind `configuration`), `max_turns_exceeded`, `budget_exceeded`, `structured_output_failed`, `agent_execution_failed`, `agent_empty_report`, `agent_no_result` (kind `research`).

#### A1.3 Claude Agent SDK usage (`sidecars/agent/src/agent.ts`)

`startResearchJob(request, config, writer, deps)` (`agent.ts:238-687`):

- **Model**: `request.model ?? config.model` (`agent.ts:245`); config model = `BLUEY_RESEARCH_MODEL ?? BLUEY_MODEL_RESEARCH ?? "claude-sonnet-5"` (`config.ts:16,128-131`).
- **Turns**: `clamp(request.maxTurns ?? config.maxTurns, 1, 64)`; default 12 (`config.ts:17-18,115-119`; `agent.ts:246`).
- **Credential gate** before spawning: `checkModelCredentials(config)` (`config.ts:147-177`) unless a `queryFn` is injected or mock mode → emits `research.failed{kind:"configuration"}` naming the variable (`ANTHROPIC_API_KEY`, `ANTHROPIC_FOUNDRY_RESOURCE`, `ANTHROPIC_FOUNDRY_API_KEY`), never its value.
- **Tool handlers** built per requested tool (`buildHandlers`, `agent.ts:290-388`), missing tool key → `missing_api_key` naming `EXA_API_KEY` / `FIRECRAWL_API_KEY`.
- **MCP server**: `createSdkMcpServer({name:"bluey", version:"0.1.0", tools: [tool("exa_search"|"firecrawl_scrape"|"document_read", desc, zodShape, handler)], timeout: 60_000})` (`agent.ts:520-565`). Tool names on the wire are `mcp__bluey__<name>`; `bareToolName()` strips the prefix for `research.toolCall`.
- **`query({prompt, options})`** (`agent.ts:577-615`) with `Options`:
  - `abortController` (job-scoped; `cancel()` aborts it)
  - `systemPrompt: buildSystemPrompt({goal, toolNames, hasDocuments})` (`system-prompt.ts:20-57`: analyst persona, method, citation rules, "never invent URLs", fact vs. inference labelling, report format, `PRIVACY_RULE`)
  - `tools: []` (removes every built-in), `allowedTools: ["mcp__bluey__…"]`, `disallowedTools: DISALLOWED_BUILTIN_TOOLS` (`agent.ts:84-107`), `permissionMode: "dontAsk"`
  - `mcpServers: { bluey: server }`, `maxTurns`, `model`
  - `cwd: mkdtempSync(tmpdir()/"bluey-agent-")` (removed in `finally`), `env: buildSubprocessEnv(baseEnv, config)`
  - `includePartialMessages: true`, `persistSession: false`, `settingSources: []`
  - `outputFormat: { type: "json_schema", schema: REPORT_OUTPUT_SCHEMA }` (`agent.ts:110-134`: `{report: string, citations: [{title,url,snippet?}]}`)
  - `pathToClaudeCodeExecutable: resolveClaudeCliPath()` (`cli-path.ts:22-30`: `BLUEY_CLAUDE_CLI` → `extractFromBunfs(embeddedClaudePath)` → SDK auto-detect), and `executable: "bun"` when `process.versions.bun` is set.
  - Prompt text: `Research query: …\n\nGoal: …\n\nInvestigate with your tools, then finish with the structured output…` (`agent.ts:608-612`).
- **`CLAUDE_CODE_USE_FOUNDRY` handling** (`config.ts:90-112`, `agent.ts:185-234`): when truthy, `FoundryConfig{resource, baseUrl, apiKey, authToken, opusModel, sonnetModel, haikuModel}` is read; `resource` falls back to the hostname of `AZURE_FOUNDRY_ENDPOINT` (`foundryResourceFromEndpoint`), `apiKey` falls back to `AZURE_FOUNDRY_API_KEY`. `buildSubprocessEnv` **deletes** every `PROVIDER_ENV_VARS` entry from the inherited env, sets `CLAUDE_AGENT_SDK_CLIENT_APP="bluey-agent/0.1.0"`, then re-adds exactly one routing: Foundry (`CLAUDE_CODE_USE_FOUNDRY=1`, `ANTHROPIC_FOUNDRY_RESOURCE` **or** `ANTHROPIC_FOUNDRY_BASE_URL` (resource wins; Claude Code rejects both), `ANTHROPIC_FOUNDRY_AUTH_TOKEN`/`ANTHROPIC_FOUNDRY_API_KEY`, pinned `ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL`) or Anthropic direct (`ANTHROPIC_API_KEY` only).
- **SDK message → event mapping** (`agent.ts:392-478,615-651`): `system/init` → `research.progress("agent session started (model …, N tool(s))")`; `stream_event` `content_block_delta/text_delta` → `research.textDelta` (sets `sawStreamText`); `assistant` `tool_use` blocks → `research.toolCall`; `assistant` `text` blocks → `textDelta` only if no stream deltas were seen (avoids double-emission); `result/success` → `structured_output` parsed with `structuredOutputSchema` (fallback: `result` text that is our JSON) → `research.completed` with `CitationStore.finalize(modelCitations)`; `result/error_*` → mapped failure codes; stream ended without `result` → `agent_no_result`; `AbortError` or `cancelRequested` → `cancelled`.
- **Timeouts**: no wall-clock cap in the sidecar itself. Per-tool: Exa 20 s (`tools/exa.ts:11`), Firecrawl 45 s (`tools/firecrawl.ts:11`), `document.request` 10 s (`tools/documents.ts:14`), MCP server 60 s. The TS caller (`src/ai/research.ts:70,284-287` `runDeepAgent`) enforces **90 s** (`DEFAULT_TIMEOUT_MS`, overridable via `EngineDeps.researchTimeoutMs`) and calls `research_deep_cancel` on expiry.
- **Cancellation chain**: `research.cancel` → `job.cancel()` → `cancelRequested=true; broker.close(reason); abortController.abort()` (`agent.ts:677-682`) → SDK aborts the CLI subprocess → `AbortError` in `run().catch` → `research.failed{cancelled}` → `done.finally` → process exit.

#### A1.4 Tools and how results/citations flow back

| tool | client | request | result to model | citation capture |
|---|---|---|---|---|
| `exa_search` | `tools/exa.ts:72-106` `POST https://api.exa.ai/search`, header `x-api-key`, body `{query, type:"auto", numResults (1..10, default 8), contents:{highlights:{maxCharacters:600}, summary:true}, startPublishedDate?}` | JSON array `[{title,url,snippet,publishedDate,author}]` as one text block | every result → `CitationStore.add({title,url,snippet})` (`agent.ts:310`) |
| `firecrawl_scrape` | `tools/firecrawl.ts:77-97` `POST https://api.firecrawl.dev/v2/scrape`, `Authorization: Bearer`, body `{url, formats:["markdown"], onlyMainContent:true}`; markdown truncated at 40 000 chars | `# title\nSource: url\n\n<markdown>` | page → `store.add({title, url: metadata.sourceURL, snippet: first 300 chars})` (`agent.ts:356`) |
| `document_read` | `tools/documents.ts` `DocumentBroker`: allow-list check **before** emitting `document.request`; 10 s timeout; text clipped to 40 000 chars | `Document <id>:\n\n<text>` | none (local docs are never citations) |

All tool failures are returned to the model as `{content:[{type:"text", text:"<label> error (<code>): …"}], isError:true}` (`toolFailure`, `agent.ts:175-181`) so the run continues; `research.progress` lines are emitted from the handlers (`exa_search: N result(s) for "…"`, `firecrawl_scrape: fetched …`, `document_read: loaded …`, `… failed for …`). `ToolErrorCode` = `missing_api_key | http_error | timeout | network_error | invalid_response | document_not_allowed | document_timeout | document_error | cancelled` (`tools/errors.ts:10-19`); `postJson` uses `AbortSignal.timeout` and never echoes keys.

**Citations** (`citations.ts`): `normalizeUrl` (lowercase host, drop fragment, strip trailing `/`, keep query); `finalize(modelCitations)` keeps model-chosen citations **only if the URL was observed by a tool**, then appends every remaining observed URL, deduped. This is the anti-hallucination guarantee the README promises. On the TS side, `runDeepAgent` turns `completed.report` into an untrusted context block and `completed.citations` into `ResearchOutcome.citations` (`src/ai/research.ts:271-282`), which `mergeCitations` puts first on the `BlueyResponse` (`src/ai/engine.ts:240-257`) and `ResponseView` renders under "Sources" (`src/features/hud/ResponseView.tsx:128-147`).

### A2. Every env var / secret the sidecar reads, and where it should originate in Rust

`loadConfig(env)` (`config.ts:114-136`) + `loadFoundryConfig` (`config.ts:90-112`) + `cli-path.ts:24`:

| variable | read by | purpose | Rust-side origin (intended) | Rust-side reality |
|---|---|---|---|---|
| `ANTHROPIC_API_KEY` | `config.ts:122` | Anthropic-direct key for Claude Code | Keychain `agent:anthropic:api_key` (`src-tauri/src/secrets/mod.rs:20` `AGENT_ANTHROPIC_KEY`; TS `SECRET_KEYS.anthropicAgentApiKey`, `commands.ts:417`) | never read into a child env — no spawner |
| `CLAUDE_CODE_USE_FOUNDRY` | `config.ts:91` | route Claude Code through Foundry | `.env` (`.env.example:44`) | `load_dotenv()` exists (`secrets/mod.rs:134-162`) but is **never called** (no `app` module) |
| `ANTHROPIC_FOUNDRY_RESOURCE` / `ANTHROPIC_FOUNDRY_BASE_URL` | `config.ts:93-94` | Foundry endpoint | `.env:47` / derived from `AZURE_FOUNDRY_ENDPOINT` (`.env:18`) | same |
| `ANTHROPIC_FOUNDRY_API_KEY` (fallback `AZURE_FOUNDRY_API_KEY`) / `ANTHROPIC_FOUNDRY_AUTH_TOKEN` | `config.ts:106-107` | Foundry credential | `.env:49,19` ; would be `provider:<azure-id>:api_key` in Keychain | same |
| `ANTHROPIC_DEFAULT_OPUS_MODEL` / `_SONNET_MODEL` / `_HAIKU_MODEL` | `config.ts:108-110` | pin alias → deployment | `.env:52-54` | same |
| `AZURE_FOUNDRY_ENDPOINT`, `AZURE_FOUNDRY_API_KEY` | `config.ts:100,106` | Foundry fallbacks | `.env:18-19` | same |
| `EXA_API_KEY` | `config.ts:124` | exa_search | Keychain `research:exa:api_key` (`secrets/mod.rs:18`; `SECRET_KEYS.exaApiKey`) — UI field in `AITab.tsx:234` | no consumer |
| `FIRECRAWL_API_KEY` | `config.ts:125` | firecrawl_scrape | Keychain `research:firecrawl:api_key` (`secrets/mod.rs:19`) — UI field `AITab.tsx:238` | no consumer |
| `BLUEY_RESEARCH_MODEL` (legacy `BLUEY_MODEL_RESEARCH`) | `config.ts:129-130` | default model | `.env:61`; the router's `ModelRoleAssignments.research` should also be passed per request via `research_run_params(request, Some(model))` (`agent.rs:23-29`) | mapper exists, unused |
| `BLUEY_AGENT_MAX_TURNS` | `config.ts:115` | default turns (≤64) | none documented | — |
| `BLUEY_AGENT_MOCK` | `config.ts:133` | mock mode | dev only | — |
| `BLUEY_CLAUDE_CLI` | `config.ts:134`, `cli-path.ts:24` | dev override for CLI path | dev only | — |
| `CLAUDE_AGENT_SDK_CLIENT_APP` | set (not read) `agent.ts:214` | telemetry tag on the CLI | — | — |

`docs/SECURITY.md:25-27` claims ".env values are imported into the Keychain on first run" — no code does this. `docs/ARCHITECTURE.md:68` lists `agent/ (research sidecar client)` and `research/ (Exa, Firecrawl)` under `src-tauri/src/` — neither directory exists.

### A3. Cleanest seam for a Gemini-backed research agent

Options considered:

**(a) `RESEARCH_BACKEND=gemini|claude` switch inside the same sidecar, Gemini function-calling loop via `@google/genai`** — recommended.

- *Protocol compatibility*: 100 %. `main.ts`, `protocol.ts`, `ProtocolWriter`, `DocumentBroker`, `CitationStore`, `system-prompt.ts` and `buildHandlers()` are backend-agnostic already. Only `startResearchJob.run()` (`agent.ts:496-652`) is Claude-specific: replace the `query()`/`Options` block with a `runGemini()` sibling and pick by `config.backend`. `ToolHandlers` (`agent.ts:51-52`) already gives you `(input) => Promise<TextToolResult>`; map each Gemini function call to `handlers[name](args)` and return the text as the function result. The zod shapes in `agent.ts:520-557` can be converted to JSON-Schema `parameters` for Gemini function declarations (reuse `z.toJSONSchema` — zod v4 is already a dependency).
- *Bundle size*: `@google/genai` is pure JS (no native CLI), so it adds a few MB to the `bun build --compile` output versus ~250–280 MB for the embedded Claude CLI (`sidecars/agent/README.md:200-203`). Because the per-arch entry files statically embed the CLI (`entry-darwin-arm64.ts:13`), a Gemini-default build could ship **without** the Claude binary by adding `entry-darwin-*-lite.ts` variants that omit the import and pass no `embeddedClaudePath` (Claude then works only with `BLUEY_CLAUDE_CLI` or an installed SDK binary). Keep the current entries for the "Claude alternate" build.
- *Testability*: the `tests/sidecar/sidecar.test.ts` harness (`makeHarness` → `startSidecar({input, output, env, deps})`) is backend-agnostic. Add a `deps.geminiClient` / `deps.generateFn` injection point mirroring `deps.queryFn` so tests can script `functionCall` → `functionResponse` turns without network. `mock.ts` (`createMockQueryFn`) scripts a Claude-shaped SDK message stream (`system/init`, `assistant tool_use`, `stream_event`, `result`) against the **real** handlers/broker/citation store; for Gemini either (i) keep mock mode on the Claude path (cheap, no change) or (ii) add `createMockGeminiTurns()` yielding `{functionCalls:[…]}` / `{text}` chunks so `BLUEY_AGENT_MOCK=1 RESEARCH_BACKEND=gemini` exercises the new loop. The mock's document round-trip (`mock.ts:117-125`) must be preserved for `document_read`.
- *Cancellation*: `@google/genai` accepts `config.abortSignal` per request; wire the existing `abortController.signal` (`agent.ts:247`) into every `generateContentStream`/`interactions.create` call and check `signal.aborted` between tool turns → same `emitCancelled()` path. No subprocess to kill, so cancellation becomes faster and more reliable than the CLI path.
- *Structured output*: Gemini supports `responseMimeType: "application/json"` + `responseSchema`/`responseJsonSchema` (`research/raw/structured-output.txt`, `additionalProperties` supported per line 936) but **function calling and JSON mode cannot be combined in the same turn** on most Gemini models — run the tool loop in text mode, then issue one final "write the report" turn with the schema (equivalent to the SDK's `outputFormat`), or parse the last text via the existing `tryParseReportJson` fallback (`agent.ts:481-492`). `CitationStore.finalize` still guarantees only observed URLs survive.
- *Turn budget*: implement `maxTurns` as the max number of model round-trips; emit `max_turns_exceeded` with the same code so `bluey-protocols::agent` and the TS caller need no change. Map Gemini `usageMetadata.promptTokenCount/candidatesTokenCount` to `usage`.
- *Streaming*: `generateContentStream` chunks with `text` → `research.textDelta`; chunks with `functionCalls` → `research.toolCall` + `progress`.
- *Which Gemini API*: the docs snapshot in `/agent/workspace/research/raw/function-calling.txt` (lines 61-99, 800-820) shows the current JS SDK favouring `ai.interactions.create({model, input, tools:[{type:'function', name, description, parameters}], previous_interaction_id})` with `interaction.steps[].type === 'function_call'` and `function_result` inputs; `api-generate-content.txt:82-92,1304-1326,1519-1521` shows `generateContent` still accepts `tools:[{functionDeclarations}]` + `toolConfig.functionCallingConfig`. Either works for the seam; `generateContentStream` gives token streaming for `textDelta`, whereas Interactions gives server-side conversation state (`previous_interaction_id`) and simpler multi-step bookkeeping. Recommend `generateContentStream` + manual loop for parity with the existing streaming events, with Interactions as a follow-up if it streams (verify `stream: true` support in the current SDK — **uncertain from the snapshot**).

**(b) Rust-native agent loop** (reqwest to `generativelanguage.googleapis.com`, tools = existing `bluey-protocols::exa/firecrawl` models). Pros: no Bun sidecar at all for Gemini, ~0 MB, no CLI extraction, keys never leave the Rust process. Cons: the sidecar's citation/allow-list/privacy logic would be re-implemented in Rust; there is currently **zero** Rust orchestration code (no `AgentManager`, no `ResearchManager`, no `commands`), so this is strictly more work; the `tests/sidecar` harness would not cover it. Reasonable as a *phase 2* once the Rust crate is actually wired.

**(c) Gemini Interactions/Managed Agents with hosted tools** (server-side deep research). Pros: least code. Cons: cannot mount Bluey's `document_read` (local, allow-listed) or the Exa/Firecrawl keys the user already has; breaks the "citations only from observed tools" guarantee; unclear cancellation/turn accounting; not testable offline. Reject for now.

**Recommendation:** (a). Concretely:
1. `config.ts`: add `backend: "gemini" | "claude"` from `RESEARCH_BACKEND` (default `"gemini"`), `geminiApiKey` from `GEMINI_API_KEY` (also accept `GOOGLE_API_KEY`), `geminiModel` from `BLUEY_RESEARCH_MODEL` when backend is gemini (default e.g. `gemini-3.8-flash`); extend `checkModelCredentials` to name `GEMINI_API_KEY`.
2. `agent.ts`: extract the Claude block into `runClaude()`, add `runGemini()` using the same `handlers`, `store`, `broker`, `progress/emit*` closures; add `PROVIDER_ENV_VARS` entry `GEMINI_API_KEY`/`GOOGLE_API_KEY` so it is stripped from any Claude subprocess env.
3. New `sidecars/agent/src/gemini.ts` (function declarations from the zod shapes, loop, usage mapping) + `tests/sidecar/gemini.test.ts`.
4. `package.json` (sidecar **and** root — `tests/sidecar/*` import sidecar sources through the root vitest config, and the root `package.json:30` already mirrors `@anthropic-ai/claude-agent-sdk` for that reason): add `@google/genai`.
5. Tests to update: `tests/sidecar/config.test.ts` (new backend/credential cases), `tests/sidecar/sidecar.test.ts` "fails with missing_api_key naming ANTHROPIC_API_KEY" (`:232-241`) becomes backend-dependent (with default gemini it should name `GEMINI_API_KEY`; keep a `RESEARCH_BACKEND=claude` variant), "runs a full research job" (`:113-156`) if mock mode gains a Gemini script; `tests/unit/ai/research.test.ts` and `tests/integration/research-flow.test.ts` are backend-agnostic and need no change. Rust `bluey-protocols/src/agent.rs` needs no change (the wire is identical); optionally forward `research.started.model` and `completed.usage` which it currently drops.

### A4. Build & embedding implications (`entry-darwin-*.ts`, `embedded-bin.d.ts`, `build-agent.sh`)

- `sidecars/agent/src/entry-darwin-arm64.ts:13` / `entry-darwin-x64.ts:6`: `import embeddedClaude from "@anthropic-ai/claude-agent-sdk-darwin-{arm64,x64}/claude" with { type: "file" }` → `runSidecarProcess({ embeddedClaudePath })`. `embedded-bin.d.ts` declares those two module specifiers as `string`. One entry per target because `bun build --compile` needs a statically analysable per-arch import.
- `scripts/build-agent.sh`: `bun install --os darwin --cpu '*'` (forces both darwin optional deps), asserts both `node_modules/@anthropic-ai/claude-agent-sdk-darwin-*/claude` exist (`:50-56`), `bun build <entry> --compile --target=bun-darwin-{arm64,x64} --outfile src-tauri/binaries/bluey-agent-<triple>` (`:75-82`), `codesign` (ad-hoc or `$BLUEY_CODESIGN_IDENTITY`). `build-agent.sh host` builds only the host arch; `scripts/ensure-sidecars.sh` runs it before `tauri dev` when the binary is missing. `src-tauri/binaries/` is git-ignored and currently empty (`.gitkeep` only).
- Runtime: `extractFromBunfs()` (`@anthropic-ai/claude-agent-sdk/extract`) copies the ~250 MB CLI to `/tmp/claude-<uid>/claude-agent-sdk-<hash>/` on first run.
- Adding `@google/genai`: pure JS, bundles fine under `bun build --compile`; must be added to `sidecars/agent/package.json` (and root `package.json` for the vitest/tsc path); no change to `build-agent.sh` is required for a mixed build. To ship a small Gemini-default binary, add lite entry files without the `with {type:"file"}` import and a `build_target` variant; keep the `pkg` existence check only for the Claude-embedding targets. `bun.lock` in the sidecar currently pins `@anthropic-ai/claude-agent-sdk@0.3.263` and its platform packages (`sidecars/agent/bun.lock`), nothing Google.

---

## PART B — Frontend completeness audit

### B0. Reference UI (from `cluely-screenshorts/*.png`, 12 viewed)

- Settings window: centred title "Cluely Settings", icon-over-label tab bar (General, Calendar, Notifications, Modes, Keybinds, Profile, Security, Billing, About), 48×48 icon tiles per row, right-aligned switches/selects/primary buttons, footer "Account and app · Reset onboarding · Log out · Quit" (`7.43.58`, `7.44.16`).
- Modes: left list with accent "New Mode" row, "General" row, group label "Looking for work", active check badge; right pane title 28px, "…" menu, "Meeting context" textarea, "Files" dashed dropzone with stacked-docs illustration, sticky "Set Active" (`7.45.18`, `7.46.12`, `7.46.25`).
- HUD idle: 690px translucent bar, "Ask anything about your screen" + ↵ chip; second row logo, blue pill ("Update Available"), icon cluster (screen, eye, mode grid, waveform), "History ↓" (`7.48.20`); tooltip "Open Settings ⌘ ," on the logo (`7.48.33`).
- HUD expanded: "← Ask follow-up … ■" header, right-aligned grey prompt pill "Assist", markdown body with inline code, floating ↓, "New Chat ⌘ R" (`7.49.44`, `7.50.03`).
- Mode menu: dark list with check on active + "Manage" (`7.49.02`); menu-bar menu: Start Listening / Hide / Enable Invisibility / View Sessions / Preferences / Quit (`7.50.45`).

Bluey's implementation matches these closely (DESIGN.md is derived from them); deviations are called out below.

### B1. Window → feature → file map

Entry: `index.html` → `src/main.tsx` → `bootstrap()` (`src/lib/tauri/bootstrap.ts`: `?window=` label, transport selection `TauriTransport` vs `MockTransport` by `__TAURI_INTERNALS__`, `initStores()`, appearance `data-*` attrs) → lazy `HudWindow` | `SettingsWindow` | `OnboardingWindow` (`src/windows/*.tsx`). Every window is wrapped in `ClerkRoot` + `TooltipProvider`; HUD and Settings in `AuthGate` (`src/lib/auth/AuthGate.tsx`: unconfigured → setup screen, `dev` mode (mock transport, no Clerk key, `auth-store.ts:38-39`) → children, signed out → `<SignIn/>` or compact HUD prompt).

**HUD (main, 690×108 → ≤620)** — `src/features/hud/`:
`HudPanel.tsx` (layout, `useAutoHeight` → `panel_set_expanded`), `HudInputRow.tsx` (`HudIdleRow`, `FollowUpHeader`), `HudToolbar.tsx` (logo, `StatePill`, screen/protection/mode/audio buttons, History ↓ → Settings→Sessions, "New Chat ⌘R"), `ModeMenu.tsx` (Radix dropdown, `modes_set_active`, Manage → Settings→Modes), `StatePill.tsx` + `state-pill.ts` (`derivePill`), `ResponseThread.tsx` (turns, auto-follow, ↓ button, `panel.scroll`), `ResponseView.tsx` (react-markdown + remark-gfm, `CodeBlock` via shiki `highlighter.ts`, lazy `MermaidDiagram`, collapsible sections, Sources), `ResponseActions.tsx` (copy answer/code, 👍/👎 + chips → `responses_feedback`, regenerate), `useAsk.ts` (engine glue), `useHudShortcuts.ts` (`shortcut.triggered`, Esc, ⌘↵, ⌘⇧↵, ⌘R), `markdown.ts` (fence buffering).

**Settings (930×690)** — `src/features/settings/SettingsShell.tsx` (tabs from `settings-nav.ts`: general, modes, keybinds, audio, screen, ai, privacy, permissions, sessions, profile, advanced, about; `?tab=` deep link), tabs in `tabs/*.tsx`, plus `ModeEditor.tsx`, `ModeFilesDropzone.tsx`, `SessionDetail.tsx`, `ProviderCard.tsx`, `SecretKeyField.tsx`, `provider-form.ts`.

**Onboarding (760×560)** — `src/features/onboarding/OnboardingFlow.tsx` STEPS: `welcome`, `sign-in`, `name` (`steps/basics.tsx`), `permissions` (`steps/permissions.tsx`, 4 sub-screens), `default-mode`, `shortcuts` (`steps/setup.tsx`), `test-screen`, `test-microphone`, `test-ai`, `ready` (`steps/tests.tsx`). Finishing sets `general.onboardingCompleted=true`, opens `main`, closes `onboarding`.

Stores (`src/stores/`): `appStore` (AppStatus from `app.state`), `chatStore` (turns/generation/phase/prepared), `settingsStore`, `modesStore`, `sessionStore`, `transcriptStore` (segments/partial/questions/levels), `panelStore`, `permissionsStore`, `devStore`, `engine.ts` (singleton `createResponseEngine()` with an "unavailable" stub fallback); `initStores.ts` wires 19 event subscriptions.

#### B1.a Settings → AI tab: exactly how providers, roles, tests, keys work

`src/features/settings/tabs/AITab.tsx`

- **Provider list**: `ai.providers.map(ProviderCard)` (`:156-168`). "Add provider" (`:150-153`) opens `ProviderDialog` (`ProviderCard.tsx:22-106`) with fields Name, Kind (`KIND_OPTIONS`, `ProviderCard.tsx:16-20`: `azure_foundry` "Azure Foundry / OpenAI", `anthropic` "Anthropic", `openai_compatible` "OpenAI-compatible" — **`mock` is deliberately absent; no Google/Gemini**), Base URL (**required**: Save disabled when empty, `:50`), API version (Azure semantics in the label), Deployments textarea (`model=deployment` per line → `draftToDeployments`, `provider-form.ts:26-37`). `providerToDraft()` defaults `kind` to `"azure_foundry"` (`provider-form.ts:15`).
- **Save** (`AITab.tsx:107-140`): add → `{ id: createId("provider"), kind, name, baseUrl, apiVersion?, deployments, enabled: true, hasApiKey: false }` appended and persisted with `update({ ai: { providers } })` (→ `settings_update` deep patch; Rust `SettingsManager.persist` upserts `ModelConfigRepository` and refreshes `has_api_key` from Keychain). Edit → same fields except `id`/`enabled`/`hasApiKey`. There is **no delete provider** action.
- **Enable switch** per card → `update({ai:{providers}})` with `enabled` flipped (`:161-165`).
- **API key** (`ProviderCard.tsx:153-156` → `SecretKeyField`): key name `SECRET_KEYS.providerApiKey(provider.id)` = `provider:<id>:api_key`. `SecretKeyField` (`SecretKeyField.tsx`) calls `bluey.secrets.has({key})` on mount, shows a password `Input` + Save → `bluey.secrets.set({key, value})` (Tauri `secrets_set`), then renders "Key saved ••••" + Replace; the key is never read back (`secrets_has`/`secrets_set`/`secrets_delete` are the only secret commands, `commands.ts:237-239`). Save errors are only `console.warn`ed (no toast/banner). Rust `SecretsStore::validate_key` (`secrets/mod.rs:41-53`) whitelists exactly `provider:*:api_key`, `research:exa:api_key`, `research:firecrawl:api_key`, `agent:anthropic:api_key`, `auth:clerk:client_token`; the mock flips `hasApiKey` when a `provider:` key is saved (`mock-transport.ts:1270-1285`).
- **Test connection** (`ProviderCard.tsx:118-128,169-171`): `bluey.ai.testConnection({ providerId })` with **no model** → Rust `AiManager::test_connection` picks `default_model_for(config)` (first role assigned to that provider, `ai/mod.rs:519-532`) or **returns `Err(config.no_model)`** (`ai/mod.rs:453-460`); `ProviderCard` only handles the `ConnectionTestResult` shape (`ok`, `latencyMs`, `error.message`) and `catch`es thrown errors with `console.warn` → a provider with no role assignment shows **nothing** when tested. Success renders "Connected · Nms"; failure renders `result.error.message` (raw technical message, no `useErrorPresenter`, no recovery button).
- **List models** (`AITab.tsx:45-57`): `ModelRoleRow` calls `bluey.ai.listModels({ providerId })` whenever the row's provider changes and feeds a `<datalist>`; errors are swallowed (`.catch(() => undefined)`), no loading indicator, no empty-state. Rust: Azure returns deployments keys + `COMMON_MODELS` (`ai/providers/azure.rs:24-35,124-136`), Anthropic `GET /v1/models` (`anthropic.rs:103-130`), OpenAI-compatible `GET /v1/models` (`openai.rs:84-101`), mock static list.
- **Model role assignments** (`AITab.tsx:18-94`): `ROLES` = default ("Main answers"), fast ("Classification, quick replies"), reasoning ("Hard problems"), vision ("Screenshots"), research ("Deep research agent"), transcription (**hint hard-codes "MAI-Transcribe-1.5 (Voice Live)"**), embedding ("Document retrieval"). Each row: provider `<Select>` (falls back to `providers[0]` when unassigned — visually implies an assignment that does not exist) + free-text model `<input list=…>` (`defaultValue`, uncontrolled; saved on **blur** via `update({ ai: { models } })`; empty string clears the role to `null`). Changing the provider re-saves with the existing model string.
- **Responses**: length/tone selects (`:177-201`). **Research**: switches `researchEnabled` ("Web search … (Exa)") and `deepResearchEnabled` ("Multi-step research with the Claude agent (public queries only)"), plus three `SecretKeyField`s: Exa (`research:exa:api_key`), Firecrawl (`research:firecrawl:api_key`), "Anthropic (agent)" (`agent:anthropic:api_key`) (`:203-248`). **Context budget** slider 4k–128k (`:250-268`).

#### B1.b Onboarding setup/test steps

`steps/setup.tsx`: `DefaultModeStep` (grid of modes → `modes_set_default`), `ShortcutsStep` (5 bindings, record via `eventToAccelerator` → `shortcuts_check_conflict` → `shortcuts_update`). **There is no provider/API-key step in onboarding.** `steps/tests.tsx` `TestAIStep` (`:93-148`): provider = the one assigned to `ai.models.default` else first `enabled` provider; if none → "Connect an AI provider … Add one in Settings → AI (Azure Foundry, Anthropic or any OpenAI-compatible endpoint)" with a button `bluey.window.open({label:"settings", route:"ai"})`; otherwise "Test connection" → `bluey.ai.testConnection({ providerId, model: ai.models.default?.model })`, result "Connected · model · Nms" / `error.message`. Thrown errors are swallowed (`console.warn`). `OnboardingFlow` always allows Continue on this step (`onReady` never called) so a failed test does not block.

#### B1.c Type definitions (verbatim)

`src/lib/types/ai.ts:26-48`:
```ts
export type AIProviderKind = "azure_foundry" | "anthropic" | "openai_compatible" | "mock";

export interface AIProviderConfig {
  id: string;
  kind: AIProviderKind;
  name: string;
  /** e.g. https://my-resource.openai.azure.com or https://api.anthropic.com */
  baseUrl: string;
  /** Azure api-version when using the legacy deployment endpoint. */
  apiVersion?: string;
  /** For Azure: map of logical model name -> deployment name (optional). */
  deployments?: Record<string, string>;
  enabled: boolean;
  /** True when a key is stored in the OS keychain (never the key itself). */
  hasApiKey: boolean;
}

export interface ModelAssignment {
  providerId: string;
  model: string;
}

export type ModelRoleAssignments = Record<ModelRole, ModelAssignment | null>;
```
`src/lib/types/ai.ts:113-119,140-158`:
```ts
export interface ConnectionTestResult {
  ok: boolean;
  providerId: string;
  model?: string;
  latencyMs?: number;
  error?: BlueyError;
}
export interface DeepResearchRequest {
  jobId: string;
  sessionId?: string;
  /** PUBLIC query only — never include private context. */
  query: string;
  goal: string;
  maxTurns?: number;
  tools: Array<"exa_search" | "firecrawl_scrape" | "document_read">;
  /** Document ids the agent may read via the document_read tool (local, private). */
  allowedDocumentIds?: string[];
}

export type DeepResearchEvent =
  | { type: "started"; jobId: string }
  | { type: "progress"; jobId: string; message: string }
  | { type: "tool_call"; jobId: string; tool: string; input: Record<string, unknown> }
  | { type: "text_delta"; jobId: string; text: string }
  | { type: "completed"; jobId: string; report: string; citations: Citation[]; totalMs: number; turns: number }
  | { type: "failed"; jobId: string; error: BlueyError };
```
`src/lib/types/mode.ts:35-42`:
```ts
export type ModelRole =
  | "default"
  | "fast"
  | "reasoning"
  | "vision"
  | "research"
  | "transcription"
  | "embedding";
```
`src/lib/types/settings.ts:40-70`:
```ts
export interface AudioSettings {
  source: "microphone" | "system" | "both";
  microphoneDeviceId?: string;
  transcriptionLanguage: "auto" | string;
  speakerIdentification: boolean;
  transcriptionProvider: TranscriptionProviderKind;
  vadSensitivity: "low" | "medium" | "high";
}
…
export interface AISettings {
  providers: AIProviderConfig[];
  models: ModelRoleAssignments;
  responseLength: ResponseLength;
  responseTone: ResponseTone;
  researchEnabled: boolean;
  deepResearchEnabled: boolean;
  embeddingsEnabled: boolean;
  proactivePreparation: boolean;
  /** Max input tokens per request (token budget). */
  contextTokenBudget: number;
}
```
`src/lib/types/transcript.ts:31-44`:
```ts
export type TranscriptionProviderKind = "apple" | "cloud_realtime" | "mock";

export interface AudioSessionConfig {
  microphone: { enabled: boolean; deviceId?: string };
  systemAudio: { enabled: boolean };
  transcription: {
    provider: TranscriptionProviderKind;
    language: "auto" | string;
    speakerIdentification: boolean;
  };
  vad: { enabled: boolean; sensitivity: "low" | "medium" | "high" };
  /** Keep raw audio according to privacy settings. */
  retainRawAudio: "never" | "until_session_end" | "custom";
}
```
Rust mirrors: `AiProviderKind { AzureFoundry, Anthropic, OpenaiCompatible, Mock }` `#[serde(rename_all="snake_case")]` (`crates/bluey-core/src/types/ai.rs:41-48`); `TranscriptionProviderKind { Apple (default), CloudRealtime, Mock }` (`types/transcript.rs:52-59`); `router::provider_supports_vision` returns true for all four kinds (`crates/bluey-core/src/router.rs:36-43`); `build_provider` matches on kind (`src-tauri/src/ai/providers/mod.rs:60-95`). A new kind must be added in all four places plus `FIXTURE_MODELS_BY_KIND` (`src/lib/tauri/mock/fixtures.ts:575-588`).

### B2. Audio tab (`src/features/settings/tabs/AudioTab.tsx`)

- Rows: Audio source (`microphone|system|both`), Microphone device (from `audio_list_devices`, `kind==="input"`; when the list is empty the Select has **no options**), Transcription language (8 hard-coded values `auto,en,es,…`), Speaker identification switch, **Transcription provider** `<Select>` with exactly two options `{ value: "apple", label: "Apple (on-device)" }` and `{ value: "cloud_realtime", label: "Cloud realtime" }` (`:126-138`), description text hard-codes "On-device Apple Speech, or cloud (MAI-Transcribe-1.5 over Voice Live)" (`:124`); VAD sensitivity; "Audio check" → `audio_test_microphone` with `LevelMeter` from `transcriptStore.levels`.
- What it shows about cloud models: **nothing dynamic**. The cloud model is whatever `ai.models.transcription` says (edited in the AI tab), and the transport is chosen in Rust by model id (`bluey-protocols::voice_live::transport_for_model`, `voice_live.rs:67-73`: MAI aliases → Voice Live, anything else → `/openai/v1/realtime?intent=transcription`). The Audio tab does not show the assigned model, its provider, whether that provider has a key, or that `docs/AI_ARCHITECTURE.md:90` admits "The WebSocket manager is not wired yet". `mock` is intentionally not offered.

### B3. Unfinished / missing / dead — findings

**Backend reality (blocks everything below in a real build)**
1. `src-tauri/src/lib.rs` = template; no `mod` for existing modules; no `commands`; `generate_handler![greet]`. `tests/integration/command-surface.test.ts` fails on both assertions.
2. Missing Rust modules referenced by `state/mod.rs` and `settings/side_effects.rs`: `agent`, `research`, `audio`, `auth`, `overlay`, `shortcuts`, `permissions`, `documents`, `platform`, `app` (incl. `app::set_log_level`, `.env` import). `load_dotenv()` is never called; SECURITY.md's env→Keychain import is unimplemented.
3. `research_search`/`research_scrape`/`research_deep_start`/`research_deep_cancel`/`research_available` have no Rust implementation; only the pure codecs exist (`bluey-protocols::exa/firecrawl/agent`).
4. Cloud STT (`cloud_realtime`) has codecs (`voice_live.rs`, `realtime.rs`) but no WebSocket manager (`AI_ARCHITECTURE.md:90`).
5. Menu bar/tray, NSPanel overlay, global shortcuts, permissions, autostart — all Rust-side, all absent (only the Swift helper exists under `src-tauri/swift/`).

**Frontend — features described in docs but missing in code**
6. **Proactive preparation is not wired end-to-end.** `engine.classify()` / `engine.prepare()` are never called outside `src/ai/engine.ts` (grep confirms). `question.detected` events land in `transcriptStore.questions` (`initStores.ts:44`) and are never consumed by any component or orchestrator. `response.prepared` → `chatStore.prepared` → pill hint → ⌘⇧↵ take path is complete on the consumer side (`StatePill.tsx:37-41`, `useAsk.ts:79-87`), but nothing produces it in a real build (README/AI_ARCHITECTURE "detected live and prepared silently").
7. **No live transcript in the HUD.** `transcriptStore.segments/partial` are read by nothing; the HUD only shows the "● Listening" pill. README promises "Live transcription … speaker labels".
8. **Deep-research progress is invisible.** `research.event` is consumed only inside `runDeepAgent`'s promise (`src/ai/research.ts:271-282`); `progress`/`tool_call`/`text_delta` are ignored; `EnginePhase` (`engine-contract.ts:55`) has no `researching` value; `ResponseThread` shows "Thinking…" for up to 90 s. Citations *do* render (`ResponseView.tsx:128-147`, also in `SessionDetail`).
9. **No "My Context" / global documents UI** (resume, CV, JD upload described in README, MODE_SYSTEM.md, `DocumentKind` in `src/lib/types/documents.ts:3-…`). Only mode-scoped files exist (`ModeFilesDropzone.tsx`, always `kind: "notes"`). Retrieval (`retrieveRelevantContext`) asks for `resume`/`job_description` kinds that the UI can never create.
10. **No Appearance settings UI.** `AppearanceSettings` (theme, opacity, width, blur, fontSize, alwaysOnTop, density, position, followActiveDisplay, reducedMotion) are consumed by `HudPanel.tsx:55-57` and `bootstrap.ts` but editable nowhere (DESIGN.md "opacity/width settings", TESTING.md "opacity/width settings"). Dead settings.
11. **Custom-mode editor is partial**: `ModeEditor.tsx` edits name, instructions, length/tone, latency, preferred model role, files; **not** `description`, `icon`, `responseSchema`, `contextRequirements`, `group` (MODE_SYSTEM.md lists "preferred output schema" and description as custom-mode fields). Built-in modes: only instructions.
12. **Shortcut `enabled` flag has no UI** (`KeybindsTab.tsx` records accelerators only; TESTING.md "disabled shortcut").
13. **Sessions are never started from the UI**: no caller of `bluey.session.start` in `src/features`; `SessionsTab` empty state says "Start an audio session from the HUD and it will show up here" — depends on Rust auto-creating sessions on `audio_start` (unverified; Rust `sessions/mod.rs` exists but is not compiled). No pause/resume/end/rename controls anywhere.
14. **Error pill has no recovery action**: `StatePill` error renders "! Something went wrong" with no button (DESIGN.md: "+ recovery button"); `app.error`, `audio.error`, `helper.status` events have **no subscribers** (no toast/banner); `useErrorPresenter` exists but is only used by `ErrorBanner` inside turns and SessionDetail.
15. **Privacy settings not enforced in TS**: `privacy.cloudAiEnabled` is only toggled (`PrivacyTab.tsx:178`), never checked by `engine.ts` before `ai_stream`; `storeRawAudio: "custom"` has no `rawAudioRetentionMinutes` input; `debugLogTranscripts` has no UI.
16. **`outputLanguage` value mismatch**: `GeneralTab.tsx:16` offers language *names* ("English", …) while Rust default is `"en"` (`types/settings.rs:133`) → the Select shows no selected option on a fresh Rust install (mock fixture uses "English").
17. **Provider UX gaps**: no delete-provider; Base URL required even where a default exists (Anthropic hard-codes `https://api.anthropic.com` fallback in Rust `anthropic.rs:31-35`); no per-kind field visibility (Azure-only "API version"/"Deployments" shown for every kind); Test connection without an assigned model silently does nothing (thrown `config.no_model`); model rows show `providers[0]` when unassigned; `defaultValue` inputs go stale when settings change remotely; listModels errors/loading invisible; no "which role uses this provider" indicator; no empty state when `ai.providers` is empty (just the header + Add button).
18. **AI tab Claude coupling**: hint "MAI-Transcribe-1.5 (Voice Live)" (`AITab.tsx:24`), "Multi-step research with the Claude agent" (`:222`), "Anthropic (agent)" key (`:241`), `TestAIStep` copy "(Azure Foundry, Anthropic or any OpenAI-compatible endpoint)" (`tests.tsx:119`).
19. **Mock-only paths**: `MockTransport.research_available` returns `deepAgent:false` (`mock-transport.ts:904`) so the deep agent path never runs in dev; `research_deep_start` mock emits `started → progress → completed` with a fixture report; `auth_fapi_fetch` returns 501; `documents_pick_files` returns two fake paths; `dev_restart_helper` fakes `helper.status`. `app_get_dev_info.agentSidecarAvailable:false`.
20. **HUD `screenEnabled` toggle is component state** (`HudPanel.tsx:22`), not persisted, not reflected in `AppStatus`; capture tooltip text "Uses Screen"/"Screen off" matches the reference.
21. **Minor**: `AboutTab` links to placeholder `https://bluey.app/help` / `support@bluey.app`; `SessionsTab` has no delete-from-list/rename; `SessionDetail` "Export markdown" copies to clipboard rather than saving a file; `ProfileTab` relies on Clerk `<UserProfile/>` (fine).

**Command surface vs. mock vs. Rust** — `COMMAND_NAMES` (120 entries, `commands.ts:286-410`) is fully implemented by `MockTransport` (asserted by `tests/ui/mock-transport.test.ts:22-28`) and by **zero** Rust commands. The UI calls nothing outside `CommandMap` (all calls go through `bluey.*`, `api.ts`). No command is declared in TS but "extra" in Rust except `greet`.

### B4. HUD assessment

- **Streaming**: `useAsk.ask()` → `chatStore.begin()` (bumps generation, adds a `streaming` turn, phase `capturing`) → `getEngine().ask(input, callbacks)`; `onPhase` → `setPhase`, `onDraft` → `applyDraft` (whole `BlueyResponse` with growing `content`), `onComplete` → `complete`, `onError` → `fail`; all generation-guarded (`chatStore.ts:79-121`). `Turn` renders `ResponseView streaming` → `splitStreamingMarkdown` withholds an open fence and shows "Writing code…" (`ResponseView.tsx:93-108`); before the first draft it shows "Reading screen…"/"Thinking…" (`ResponseThread.tsx:35-39`). Engine side: `CodeFenceBuffer`/`visibleWithHeldFences` and `extractPartialStringField(accumulated,"content")` for structured JSON streams (`src/ai/engine.ts:385-410`, `src/ai/stream.ts`). Stop: `FollowUpHeader` ■ / Esc → `useAsk.stop()` → `markCancelled` + `handle.cancel()` → `ai_cancel`. Height: `useAutoHeight` → `panel_set_expanded`. Verified by `tests/ui/hud.test.tsx`.
- **Transcript / listening**: only `derivePill` (`state-pill.ts:13-26`: error > reading > thinking > prepared > listening > idle) and the pulsing dot on the audio button (`HudToolbar.tsx:104-106`). No transcript text, no speaker labels, no detected-question chips.
- **Prepared response (⌘⇧↵)**: `useHudShortcuts` maps `shortcut.triggered{generate_response}` and local ⌘⇧↵ to `generateOrTakePrepared` → `engine.takePrepared() ?? chatStore.prepared` → `showResponse(prepared, prepared.prompt ?? "Suggestion")`; otherwise `ask({trigger:"shortcut_generate", promptLabel:"Suggested response"})`. Complete on the UI side (`tests/ui/hud.test.tsx:96-110`), but no producer (see B3.6) and `takePrepared()` without an event id pops the *newest* prepared response regardless of which question is on screen.
- **Research/citations**: citations render as a numbered "Sources" list with `openExternal`; deep-research progress/tool calls are not rendered (B3.8); `research.toolCall` input is never shown.

### B5. Prioritized list — "Frontend gaps to close for a fully built UI"

P0 (product cannot work without them)
1. Wire the transcript → `engine.classify()` → `engine.prepare()` → `response.prepared` loop (new `src/stores/proactive.ts` or hook in `initStores.ts` subscribing to `transcript.final`; respect `ai.proactivePreparation`; consume `transcriptStore.questions`).
2. Live transcript view in the HUD (rolling last N segments + partial, speaker label + confidence, in the expanded body or a collapsible strip) from `transcriptStore` (`src/features/hud/`).
3. Global "My Context" documents UI (resume/CV/JD/notes upload with `kind`, list/delete/reindex) — new Settings tab or section; reuse `ModeFilesDropzone` with a kind picker; `documents_add` scope `global`.
4. Error surfacing: subscribe to `app.error`, `audio.error`, `helper.status` → toast/`ErrorBanner` with `useErrorPresenter`; add the recovery button to the error pill (`StatePill.tsx`) using `AppStatus.error.recovery`; stop swallowing thrown errors in `ProviderCard`, `TestAIStep`, `SecretKeyField`, `AudioTab`.
5. Session controls: start/pause/resume/end (HUD toolbar or menu) or document that Rust auto-starts on `audio_start`; show active session in the pill/tooltips.

P1
6. Deep-research progress UI: add `EnginePhase "researching"`, forward `research.event` progress/tool_call into the current turn (e.g. "Searching: …", "Reading example.com…"), show tool call count and a cancel that calls `research_deep_cancel`.
7. Appearance tab (theme, opacity, width, blur, font size, density, position, follow display, reduced motion).
8. Provider UX: delete provider; per-kind fields; empty state; role→provider badges; controlled model inputs; listModels loading/error; connection test with explicit model pick when none assigned.
9. Custom mode editor completeness (description, icon, `responseSchema`, `contextRequirements`, group); shortcut enable/disable toggle; `rawAudioRetentionMinutes` input; `outputLanguage` code/label reconciliation.
10. Onboarding: key step (see B6), block Continue on failed AI test or show "skip", persist `screenEnabled`.

P2
11. About links, export-to-file for sessions, session rename/delete from list, HUD "History" popover (recent responses) instead of jumping to Settings.

### B6. UI changes required for the Gemini-default migration

1. **Provider kind picker**: add `"google_gemini"` to `AIProviderKind` (`src/lib/types/ai.ts:26`), Rust `AiProviderKind::GoogleGemini` (`types/ai.rs:43-48`, serde `google_gemini`), `router::provider_supports_vision` (true), `build_provider` arm, `KIND_OPTIONS` in `ProviderCard.tsx:16-20` ("Google Gemini (AI Studio)" **first**), `providerToDraft` default kind → `google_gemini` (`provider-form.ts:15`), `FIXTURE_MODELS_BY_KIND.google_gemini` + a default Gemini provider in `createDefaultSettings()` (`fixtures.ts:311-404`) so `bun run dev` exercises it. Per-kind form: hide "API version" and "Deployments" for Gemini; make Base URL optional with placeholder `https://generativelanguage.googleapis.com` (relax the Save guard `ProviderCard.tsx:50`).
2. **Google AI Studio key field**: `SecretKeyField` label/aria "Google AI Studio API key", placeholder `AIza…`, helper link "Get a key at aistudio.google.com/apikey" (`openExternal`), still stored as `provider:<id>:api_key`. Optionally a second global key slot is unnecessary — one provider row is the single key.
3. **Default model presets per role**: a "Use recommended Gemini models" button (AITab Models header) writing `ModelRoleAssignments` for the Gemini provider. From the docs snapshot (`research/raw/models.txt` lines 37-52 — **parent to confirm current ids**): default `gemini-3.8-flash`, fast `gemini-3.5-flash-lite` (or `gemini-3.1-flash-lite`), reasoning `gemini-3.1-pro-preview` (or `gemini-3.8-flash` with `thinkingLevel: high`), vision `gemini-3.8-flash`, research `gemini-3.8-flash`, transcription `gemini-3.5-transcribe-live`, embedding `gemini-embedding-2` (`embeddings.txt:16`). Update `ROLES[].hint` strings (`AITab.tsx:18-26`) to be provider-neutral (drop "MAI-Transcribe-1.5 (Voice Live)").
4. **Transcription provider option**: add `"gemini_live"` to `TranscriptionProviderKind` (TS `transcript.ts:31`, Rust `transcript.rs:52-59`) and `AudioTab` option `{ value: "gemini_live", label: "Gemini Live (cloud)" }`; description becomes dynamic: show the transcription-role model and provider (`settings.ai.models.transcription`), a warning when that provider has no key, and the Gemini limits (16 kHz PCM16 via `sendRealtimeInput`, `inputAudioTranscription`, **10-minute session cap for `gemini-3.5-transcribe-live`** → Rust must rotate sessions like the Apple 55 s rotation; batch `gemini-3.5-transcribe` supports diarization/word timestamps, live does not — `gemini-3.5-transcribe.txt`). Alternatively keep `cloud_realtime` and route by model id in Rust (like `voice_live::transport_for_model`), but an explicit kind avoids the AudioTab ambiguity.
5. **Onboarding one-key path**: insert a `connect-ai` step after `name` (or replace `test-ai`): paste key → if no Gemini provider exists create `{kind:"google_gemini", name:"Google Gemini", baseUrl:""}` → `secrets_set` → apply presets → auto `testConnection({providerId, model: default})`; "I'll use another provider" link → Settings→AI; "Skip" allowed. `TestAIStep` copy: replace the Azure/Anthropic sentence with "Add your Google AI Studio key (recommended) or another provider".
6. **Connection test wording/mapping**: success "Connected · gemini-3.8-flash · 412 ms". Failures via `useErrorPresenter` (not raw `error.message`): 400 `API_KEY_INVALID` → configuration "Invalid Google AI Studio key" + Configure provider; 403 `PERMISSION_DENIED` → configuration; 404 model → configuration "Model not available for this key — pick another"; **429 `RESOURCE_EXHAUSTED`** → Rust `map_http_status` already yields `network.http_429` + Retry (`ai/providers/mod.rs:108-111`); add a code-specific message in `useErrorPresenter` ("Gemini rate limit reached (free tier). Wait a moment or check your quota in AI Studio") and honour `retryDelay` from the error body if surfaced in `details`; 503 `UNAVAILABLE` → network Retry. Run the test with `maxOutputTokens: 8` as today (`ai/mod.rs:473-483`).
7. **Model list from `models.list`**: Rust Gemini adapter `list_models` → `GET /v1beta/models?pageSize=…` with `nextPageToken` pagination (`api-models.txt:88-119`), strip the `models/` prefix, and return a role-filterable shape (or filter client-side by name: `*embedding*` for embedding, `*transcribe*` for transcription, exclude `*image*`, `*tts*`, `*live*` for chat roles). `ModelRoleRow` datalist should filter per role and show a loading spinner / "Couldn't load models" text.
8. **Cost/quota hints**: per-role helper text ("Flash-Lite is cheapest; Pro for hard problems"), a "Free tier" badge on the Gemini card with a link to AI Studio rate limits, and token usage from the `usage` chunk (already tracked in `ResponseMetrics`) shown in `AdvancedTab`/dev overlay per provider.
9. **Research section**: rename "Anthropic (agent)" → "Research agent backend" `<Select>` `{gemini (default), claude}` mapped to `RESEARCH_BACKEND`; show that Gemini research reuses the Gemini provider key (no extra secret) and that Claude requires `agent:anthropic:api_key` (or Foundry); copy "Multi-step research with the Claude agent" → "Multi-step research agent (Gemini by default; public queries only)". Keep Exa/Firecrawl fields.
10. **Alternates switchable via env + Settings**: the provider list already supports multiple providers; add a "Default provider" concept = the provider bound to the `default` role (badge on the card) and make `.env` bootstrap (once implemented in Rust) create the Gemini provider from `GEMINI_API_KEY`/`GOOGLE_API_KEY` alongside the existing `AZURE_FOUNDRY_*`/`ANTHROPIC_*` handling (`.env.example` needs a Gemini block).
11. **Tests to add/update**: `tests/ui/settings.test.tsx` (ProviderDialog kind picker default = Gemini, per-kind fields), `tests/ui/onboarding.test.tsx` (new key step), `tests/ui/mock-transport.test.ts` (Gemini fixtures), `tests/unit/ai/*` unaffected; command-surface test once Rust exists.
