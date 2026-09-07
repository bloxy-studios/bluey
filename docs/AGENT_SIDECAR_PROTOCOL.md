# Bluey Research Agent Sidecar Protocol (Bun/Node ⇄ Rust)

Deep, multi-step research runs in a separate **agent sidecar** (`bluey-agent`,
TypeScript in `sidecars/agent/`, compiled with `bun build --compile` into
`src-tauri/binaries/bluey-agent-<target-triple>`). It uses the **Claude Agent SDK**
(`@anthropic-ai/claude-agent-sdk`) with a tightly scoped tool set. The live assistant's
normal answers never go through this process — only the `deep_agent` branch of the
Research Router.

## Security model
* Rust spawns the sidecar per job with credentials in **environment variables**
  (`ANTHROPIC_API_KEY`, `EXA_API_KEY`, `FIRECRAWL_API_KEY`) read from the OS keychain.
  The WebView never sees them.
* The agent gets **only** the custom in-process MCP tools listed in the request:
  `exa_search`, `firecrawl_scrape`, `document_read`. Built-in tools (Bash, Read, Write,
  Edit, WebFetch, WebSearch, …) are disallowed via `allowedTools`/`disallowedTools` and
  `permissionMode` is set so nothing else can be invoked. `cwd` is an empty temp dir.
* `document_read` can only read the document ids in `allowedDocumentIds`; Rust serves
  the text through the `document.request` round-trip so the sidecar never touches SQLite.
* The public query/goal must not include private context. The Research Router in TS
  separates private context (kept local) from public queries (see `SECURITY.md`).

## Transport
stdio JSON Lines, same envelope as the native helper (`id`/`method`/`params`,
`id`/`result`, `id`/`error`, `event`/`data`). One job per process; the process exits
when the job completes, fails, or is cancelled.

## Methods (Rust → agent)
| method | params | result |
|---|---|---|
| `research.run` | `DeepResearchRequest` (see below) | `{ "accepted": true }` immediately; progress via events |
| `research.cancel` | `{ "jobId" }` | `{ "cancelled": true }` |
| `document.response` | `{ "requestId", "documentId", "text"?: string, "error"?: string }` | – (answers a `document.request` event) |

```jsonc
// DeepResearchRequest (mirrors src/lib/types/ai.ts)
{
  "jobId": "job-…", "query": "public search query", "goal": "what the report must answer",
  "maxTurns": 12, "tools": ["exa_search", "firecrawl_scrape", "document_read"],
  "allowedDocumentIds": ["doc-1"], "model": "claude-sonnet-4-…"
}
```

## Events (agent → Rust)
| event | data |
|---|---|
| `research.started` | `{ "jobId", "model" }` |
| `research.progress` | `{ "jobId", "message" }` |
| `research.toolCall` | `{ "jobId", "tool", "input" }` |
| `research.textDelta` | `{ "jobId", "text" }` |
| `research.completed` | `{ "jobId", "report": markdown, "citations": [ { "title", "url", "snippet"? } ], "turns": n, "totalMs": n, "usage": { "inputTokens", "outputTokens" } }` |
| `research.failed` | `{ "jobId", "error": { "code", "message", "kind": "research" } }` |
| `document.request` | `{ "requestId", "documentId" }` → Rust replies with `document.response` |

Rust maps these onto `DeepResearchEvent` and emits `research.event` to the frontend.

## Implementation notes (as built in `sidecars/agent`)
* `document.response` is fire-and-forget: no response frame on success; an error frame is only
  sent for malformed params.
* Closing stdin does **not** cancel a running job (shell pipes close immediately); cancel with
  `research.cancel` or SIGTERM/SIGINT.
* `research.failed.error.kind` is one of `research`, `cancelled`, `configuration` (missing
  keys — the message names the env var, never its value).
* Citations returned by the model are validated against URLs actually observed through the
  tools; invented URLs are dropped and remaining observed sources are appended (deduped).
* Environment: `ANTHROPIC_API_KEY`, `EXA_API_KEY`, `FIRECRAWL_API_KEY`, `BLUEY_RESEARCH_MODEL`
  (default `claude-sonnet-5`), `BLUEY_AGENT_MAX_TURNS` (default 12), `BLUEY_AGENT_MOCK=1` for
  the network-free mock mode, `BLUEY_CLAUDE_CLI` to point at a CLI binary in dev.
* The compiled binary embeds the platform-specific Claude CLI (`entry-darwin-arm64.ts` /
  `entry-darwin-x64.ts`) and extracts it with `extractFromBunfs` at runtime.
