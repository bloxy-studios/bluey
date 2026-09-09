/**
 * The research job runner: builds the scoped tool handlers, runs the selected
 * backend (`gemini` function-calling loop or the Claude Agent SDK `query()`)
 * and maps its output onto the sidecar protocol events.
 *
 * Both backends share the tool handlers, the citation store (only tool-observed
 * URLs survive), the document broker and the system prompt; only the model
 * loop differs (`runGemini` in ./gemini.ts, `runClaude` below).
 *
 * Security posture of the Claude backend (see docs/AGENT_SIDECAR_PROTOCOL.md):
 *  - `tools: []` removes every built-in tool from the agent;
 *  - `allowedTools` lists only the requested `mcp__bluey__*` tools;
 *  - `disallowedTools` re-bans the built-ins (belt and braces);
 *  - `permissionMode: "dontAsk"` denies anything not pre-approved, never prompts;
 *  - `cwd` is a fresh empty temp dir; `settingSources: []`, `persistSession: false`;
 *  - `env` REPLACES the subprocess environment (SDK semantics), so we pass a
 *    controlled copy with the model credentials injected (see
 *    `buildSubprocessEnv`): ANTHROPIC_API_KEY for Anthropic direct, or the
 *    CLAUDE_CODE_USE_FOUNDRY / ANTHROPIC_FOUNDRY_* set for Microsoft Foundry.
 *    Gemini keys are stripped from it — Claude Code never needs them.
 */

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createSdkMcpServer, query, tool, type Options } from "@anthropic-ai/claude-agent-sdk";
import { z } from "zod";

import { CitationStore } from "./citations";
import { resolveClaudeCliPath } from "./cli-path";
import {
  checkClaudeCliAvailable,
  checkModelCredentials,
  type AgentConfig,
  type BuildVariant,
} from "./config";
import { createGeminiGenerate, GeminiRunError, mapGeminiError, runGemini, type GenerateFn } from "./gemini";
import { createMockGeminiGenerate, createMockQueryFn } from "./mock";
import {
  type DeepResearchRequest,
  type DocumentResponseParams,
  type ProtocolWriter,
  type ResearchToolName,
  type WireCitation,
} from "./protocol";
import { buildSystemPrompt } from "./system-prompt";
import { TOOL_INPUT_SCHEMAS, TOOL_SPECS, type ToolInput } from "./tool-specs";
import { DocumentBroker } from "./tools/documents";
import { ToolError } from "./tools/errors";
import { createExaClient, type ExaClient } from "./tools/exa";
import { createFirecrawlClient, type FirecrawlClient } from "./tools/firecrawl";

// ── Public types ─────────────────────────────────────────────────────────────

export interface TextToolResult {
  /** MCP CallToolResult allows passthrough keys — mirror its index signature. */
  [key: string]: unknown;
  content: Array<{ type: "text"; text: string }>;
  isError?: boolean;
}

export type ToolHandler = (input: Record<string, unknown>) => Promise<TextToolResult>;
export type ToolHandlers = Partial<Record<ResearchToolName, ToolHandler>>;

/**
 * Anything that behaves like the SDK's `query()` — the mock and the tests
 * provide alternatives. Messages are iterated as `unknown` and narrowed
 * structurally so mocks don't have to reproduce the full SDKMessage union.
 */
export type QueryFn = (params: { prompt: string; options: Options }) => AsyncIterable<unknown>;

export interface AgentRunDeps {
  /** Claude backend injection (tests / mock mode). */
  queryFn?: QueryFn;
  /** Gemini backend injection (tests / mock mode). */
  generateFn?: GenerateFn;
  exaClient?: ExaClient;
  firecrawlClient?: FirecrawlClient;
  /** Passed by the compiled per-target entrypoint (embedded CLI binary). */
  embeddedClaudePath?: string;
  /**
   * How this binary was built (`lite` = no embedded Claude CLI). Set by the
   * compiled entrypoints; undefined when running un-bundled in dev, where the
   * SDK may still find the CLI in node_modules.
   */
  buildVariant?: BuildVariant;
  /** Base environment (defaults to process.env). */
  env?: Record<string, string | undefined>;
}

export interface ResearchJobHandle {
  jobId: string;
  done: Promise<void>;
  cancel(reason?: string): void;
  handleDocumentResponse(params: DocumentResponseParams): boolean;
}

// ── Constants ────────────────────────────────────────────────────────────────

export const MCP_SERVER_NAME = "bluey";
const MCP_TOOL_PREFIX = `mcp__${MCP_SERVER_NAME}__`;

/** Belt and braces: built-ins are already removed via `tools: []`. */
export const DISALLOWED_BUILTIN_TOOLS = [
  "Bash",
  "BashOutput",
  "KillShell",
  "Read",
  "Write",
  "Edit",
  "MultiEdit",
  "NotebookEdit",
  "NotebookRead",
  "WebFetch",
  "WebSearch",
  "Glob",
  "Grep",
  "LS",
  "Task",
  "TodoWrite",
  "ExitPlanMode",
  "Skill",
  "SlashCommand",
  "AskUserQuestion",
  "ListMcpResources",
  "ReadMcpResource",
];

/** json_schema structured output: { report, citations } */
export const REPORT_OUTPUT_SCHEMA: Record<string, unknown> = {
  type: "object",
  additionalProperties: false,
  required: ["report", "citations"],
  properties: {
    report: {
      type: "string",
      description: "The full research report in Markdown (headings, citations inline).",
    },
    citations: {
      type: "array",
      description: "Sources actually used. Only URLs returned by tool calls.",
      items: {
        type: "object",
        additionalProperties: false,
        required: ["title", "url"],
        properties: {
          title: { type: "string" },
          url: { type: "string" },
          snippet: { type: "string" },
        },
      },
    },
  },
};

const structuredOutputSchema = z.object({
  report: z.string(),
  citations: z
    .array(
      z.object({
        title: z.string(),
        url: z.string(),
        snippet: z.string().optional(),
      }),
    )
    .optional(),
});

type StructuredReport = z.infer<typeof structuredOutputSchema>;

const DOCUMENT_TEXT_MAX_CHARS = 40_000;

// ── Structural narrowing helpers (SDK messages are handled as `unknown`) ────

function rec(value: unknown): Record<string, unknown> | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function str(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function num(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

export function bareToolName(name: string): string {
  return name.startsWith(MCP_TOOL_PREFIX) ? name.slice(MCP_TOOL_PREFIX.length) : name;
}

function toolText(text: string): TextToolResult {
  return { content: [{ type: "text", text }] };
}

function toolFailure(err: unknown, label: string): TextToolResult {
  const message =
    err instanceof ToolError
      ? `${label} error (${err.code}): ${err.message}`
      : `${label} error: ${err instanceof Error ? err.message : String(err)}`;
  return { content: [{ type: "text", text: message }], isError: true };
}

/**
 * Validate a tool call's arguments against the shared zod shape. The Claude
 * SDK does this inside `tool()` before invoking the handler; Gemini function-
 * call arguments arrive unchecked, so every handler validates itself. Invalid
 * input becomes a tool error the model can act on — a tool is never run with
 * defaults substituted for missing or malformed arguments.
 */
function parseToolInput<Name extends ResearchToolName>(
  name: Name,
  input: Record<string, unknown>,
): { ok: true; data: ToolInput<Name> } | { ok: false; failure: TextToolResult } {
  const parsed = TOOL_INPUT_SCHEMAS[name].safeParse(input);
  if (parsed.success) return { ok: true, data: parsed.data as ToolInput<Name> };
  const detail = parsed.error.issues
    .map((issue) => `${issue.path.join(".") || "input"}: ${issue.message}`)
    .join("; ");
  return { ok: false, failure: toolFailure(new ToolError("invalid_arguments", detail), name) };
}

const URL_ECHO_MAX_CHARS = 120;

/**
 * Only web pages can be scraped: `new URL()` happily accepts `file:`,
 * `javascript:` or `data:` URLs, so the scheme is checked explicitly. Returns
 * the problem to report to the model, or undefined when the URL is fine.
 */
function invalidHttpUrl(url: string): string | undefined {
  const shown = url.length > URL_ECHO_MAX_CHARS ? `${url.slice(0, URL_ECHO_MAX_CHARS)}…` : url;
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return `"${shown}" is not a valid URL`;
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    return `"${shown}" is not an http(s) URL — only http:// and https:// pages can be scraped`;
  }
  return undefined;
}

const ERROR_MESSAGE_MAX_CHARS = 200;

/**
 * Make arbitrary SDK/runtime error text safe for the protocol: drop anything
 * that looks like a JSON response body (status-less ApiErrors embed them, and
 * bodies can echo prompt text), redact API keys and `key=` / `token=` values,
 * collapse whitespace and cap the length. Known error codes never come through
 * here — only the generic `agent_execution_failed` fallback and the free-text
 * detail of Claude Code result errors do.
 */
export function sanitizeErrorMessage(message: string): string {
  let text = message;
  const open = text.indexOf("{");
  const close = text.lastIndexOf("}");
  if (open !== -1 && close > open) {
    text = `${text.slice(0, open)}[response body removed]${text.slice(close + 1)}`;
  }
  text = text
    .replace(/AIza[0-9A-Za-z_-]{20,}/g, "[redacted]")
    .replace(/sk-ant-[0-9A-Za-z_-]{10,}/g, "[redacted]")
    .replace(/\b((?:api[_-]?)?key|token|authorization)=[^\s&"'`]+/gi, "$1=[redacted]")
    .replace(/\bBearer\s+[A-Za-z0-9._~+/=-]{8,}/gi, "Bearer [redacted]")
    .replace(/\s+/g, " ")
    .trim();
  if (text.length > ERROR_MESSAGE_MAX_CHARS) {
    text = `${text.slice(0, ERROR_MESSAGE_MAX_CHARS - 1)}…`;
  }
  return text || "unknown error";
}

/** Fallback: final text that happens to be our JSON shape. */
export function tryParseReportJson(text: string): StructuredReport | undefined {
  const trimmed = text.trim();
  if (!trimmed.startsWith("{")) return undefined;
  try {
    const parsed = structuredOutputSchema.safeParse(JSON.parse(trimmed));
    return parsed.success ? parsed.data : undefined;
  } catch {
    return undefined;
  }
}

// ── Subprocess environment ───────────────────────────────────────────────────

/**
 * Variables that select a model endpoint/credential. All of them are cleared
 * from the Claude Code subprocess environment; the Claude routing is then
 * re-added from `config`. Gemini keys never reach the CLI.
 */
const PROVIDER_ENV_VARS = [
  "ANTHROPIC_API_KEY",
  "ANTHROPIC_AUTH_TOKEN",
  "ANTHROPIC_BASE_URL",
  "CLAUDE_CODE_USE_FOUNDRY",
  "ANTHROPIC_FOUNDRY_RESOURCE",
  "ANTHROPIC_FOUNDRY_BASE_URL",
  "ANTHROPIC_FOUNDRY_API_KEY",
  "ANTHROPIC_FOUNDRY_AUTH_TOKEN",
  "ANTHROPIC_DEFAULT_OPUS_MODEL",
  "ANTHROPIC_DEFAULT_SONNET_MODEL",
  "ANTHROPIC_DEFAULT_HAIKU_MODEL",
  "GEMINI_API_KEY",
  "GOOGLE_API_KEY",
] as const;

/**
 * Build the environment handed to the Claude Code subprocess. The SDK's `env`
 * option REPLACES the child environment, so `baseEnv` (PATH, HOME, …) is kept
 * and only the provider selection is rewritten from `config` — stray values
 * (for example an `ANTHROPIC_BASE_URL` pointing somewhere else while Foundry is
 * on, or both Foundry resource *and* base URL, which Claude Code rejects) are
 * cleared first so the effective routing is exactly what `loadConfig` decided.
 */
export function buildSubprocessEnv(
  baseEnv: Record<string, string | undefined>,
  config: AgentConfig,
): Record<string, string | undefined> {
  const env: Record<string, string | undefined> = {
    ...baseEnv,
    CLAUDE_AGENT_SDK_CLIENT_APP: "bluey-agent/0.1.0",
  };
  for (const name of PROVIDER_ENV_VARS) delete env[name];

  const foundry = config.foundry;
  if (foundry) {
    env["CLAUDE_CODE_USE_FOUNDRY"] = "1";
    if (foundry.resource) env["ANTHROPIC_FOUNDRY_RESOURCE"] = foundry.resource;
    else if (foundry.baseUrl) env["ANTHROPIC_FOUNDRY_BASE_URL"] = foundry.baseUrl;
    if (foundry.authToken) env["ANTHROPIC_FOUNDRY_AUTH_TOKEN"] = foundry.authToken;
    if (foundry.apiKey) env["ANTHROPIC_FOUNDRY_API_KEY"] = foundry.apiKey;
    // Pin the alias → deployment mapping; without it Claude Code falls back to
    // its built-in Foundry defaults, which may not be deployed in the resource.
    if (foundry.opusModel) env["ANTHROPIC_DEFAULT_OPUS_MODEL"] = foundry.opusModel;
    if (foundry.sonnetModel) env["ANTHROPIC_DEFAULT_SONNET_MODEL"] = foundry.sonnetModel;
    if (foundry.haikuModel) env["ANTHROPIC_DEFAULT_HAIKU_MODEL"] = foundry.haikuModel;
  } else if (config.anthropicApiKey) {
    env["ANTHROPIC_API_KEY"] = config.anthropicApiKey;
  }
  return env;
}

// ── Job runner ───────────────────────────────────────────────────────────────

export function startResearchJob(
  request: DeepResearchRequest,
  config: AgentConfig,
  writer: ProtocolWriter,
  deps: AgentRunDeps = {},
): ResearchJobHandle {
  const jobId = request.jobId;
  const model = request.model ?? config.model;
  const maxTurns = Math.max(1, Math.min(request.maxTurns ?? config.maxTurns, 64));
  const abortController = new AbortController();
  const broker = new DocumentBroker(writer, request.allowedDocumentIds ?? []);
  const store = new CitationStore();
  const startedAt = Date.now();

  let cancelRequested = false;
  let finished = false;
  let tmpDir: string | undefined;

  const progress = (message: string): void => {
    if (!finished) writer.event("research.progress", { jobId, message });
  };

  const emitFailed = (code: string, message: string, kind: string): void => {
    if (finished) return;
    finished = true;
    writer.event("research.failed", { jobId, error: { code, message, kind } });
  };

  const emitCompleted = (data: {
    report: string;
    citations: WireCitation[];
    turns: number;
    usage: { inputTokens: number; outputTokens: number };
  }): void => {
    if (finished) return;
    finished = true;
    writer.event("research.completed", {
      jobId,
      report: data.report,
      citations: data.citations,
      turns: data.turns,
      totalMs: Date.now() - startedAt,
      usage: data.usage,
    });
  };

  const emitCancelled = (): void => {
    emitFailed("cancelled", "research job was cancelled", "cancelled");
  };

  // ── Tool handlers (shared by both backends and the mocks) ─────────────────

  function buildHandlers(): ToolHandlers | { missingKey: string } {
    const handlers: ToolHandlers = {};

    if (request.tools.includes("exa_search")) {
      let client = deps.exaClient;
      if (!client && !config.mockMode) {
        if (!config.exaApiKey) return { missingKey: "EXA_API_KEY" };
        client = createExaClient({ apiKey: config.exaApiKey });
      }
      const exa = client;
      handlers.exa_search = async (input) => {
        if (!exa)
          return toolFailure(new ToolError("missing_api_key", "EXA_API_KEY is not set"), "exa_search");
        const parsed = parseToolInput("exa_search", input);
        if (!parsed.ok) return parsed.failure;
        const { query: queryText, numResults, startPublishedDate } = parsed.data;
        try {
          const results = await exa.search({ query: queryText, numResults, startPublishedDate });
          for (const r of results) store.add({ title: r.title, url: r.url, snippet: r.snippet });
          progress(`exa_search: ${results.length} result(s) for "${queryText}"`);
          return toolText(
            JSON.stringify(
              results.map((r) => ({
                title: r.title,
                url: r.url,
                snippet: r.snippet,
                publishedDate: r.publishedDate,
                author: r.author,
              })),
              null,
              1,
            ),
          );
        } catch (err) {
          progress(`exa_search failed for "${queryText}"`);
          return toolFailure(err, "exa_search");
        }
      };
    }

    if (request.tools.includes("firecrawl_scrape")) {
      let client = deps.firecrawlClient;
      if (!client && !config.mockMode) {
        if (!config.firecrawlApiKey) return { missingKey: "FIRECRAWL_API_KEY" };
        client = createFirecrawlClient({ apiKey: config.firecrawlApiKey });
      }
      const firecrawl = client;
      handlers.firecrawl_scrape = async (input) => {
        if (!firecrawl)
          return toolFailure(
            new ToolError("missing_api_key", "FIRECRAWL_API_KEY is not set"),
            "firecrawl_scrape",
          );
        const parsed = parseToolInput("firecrawl_scrape", input);
        if (!parsed.ok) return parsed.failure;
        const { url } = parsed.data;
        const urlProblem = invalidHttpUrl(url); // validate before hitting the API
        if (urlProblem) {
          return toolFailure(new ToolError("invalid_arguments", urlProblem), "firecrawl_scrape");
        }
        try {
          const page = await firecrawl.scrape(url);
          store.add({ title: page.title, url: page.url, snippet: page.markdown.slice(0, 300) });
          progress(`firecrawl_scrape: fetched ${page.url}${page.truncated ? " (truncated)" : ""}`);
          const header = `# ${page.title ?? page.url}\nSource: ${page.url}\n\n`;
          return toolText(header + page.markdown);
        } catch (err) {
          progress(`firecrawl_scrape failed for ${url}`);
          return toolFailure(err, "firecrawl_scrape");
        }
      };
    }

    if (request.tools.includes("document_read")) {
      handlers.document_read = async (input) => {
        const parsed = parseToolInput("document_read", input);
        if (!parsed.ok) return parsed.failure;
        const { documentId } = parsed.data;
        try {
          const text = await broker.read(documentId);
          progress(`document_read: loaded "${documentId}" (${text.length} chars)`);
          const clipped =
            text.length > DOCUMENT_TEXT_MAX_CHARS
              ? `${text.slice(0, DOCUMENT_TEXT_MAX_CHARS)}\n\n[… document truncated by bluey-agent …]`
              : text;
          return toolText(`Document ${documentId}:\n\n${clipped}`);
        } catch (err) {
          if (!(err instanceof ToolError && err.code === "document_not_allowed")) {
            progress(`document_read failed for "${documentId}"`);
          }
          return toolFailure(err, "document_read");
        }
      };
    }

    return handlers;
  }

  // ── Claude backend: SDK message mapping ───────────────────────────────────

  let sawStreamText = false;

  function handleAssistantMessage(m: Record<string, unknown>): void {
    if (m["parent_tool_use_id"] != null) return; // subagent traffic (never expected here)
    const message = rec(m["message"]);
    const content = Array.isArray(message?.["content"]) ? (message?.["content"] as unknown[]) : [];
    for (const rawBlock of content) {
      const block = rec(rawBlock);
      if (!block) continue;
      if (block["type"] === "tool_use") {
        const name = str(block["name"]) ?? "unknown";
        writer.event("research.toolCall", {
          jobId,
          tool: bareToolName(name),
          input: rec(block["input"]) ?? {},
        });
      } else if (block["type"] === "text" && !sawStreamText) {
        const text = str(block["text"]);
        if (text) writer.event("research.textDelta", { jobId, text });
      }
    }
  }

  function handleStreamEvent(m: Record<string, unknown>): void {
    if (m["parent_tool_use_id"] != null) return;
    const event = rec(m["event"]);
    if (event?.["type"] !== "content_block_delta") return;
    const delta = rec(event["delta"]);
    if (delta?.["type"] !== "text_delta") return;
    const text = str(delta["text"]);
    if (!text) return;
    sawStreamText = true;
    writer.event("research.textDelta", { jobId, text });
  }

  function usageFrom(m: Record<string, unknown>): { inputTokens: number; outputTokens: number } {
    const usage = rec(m["usage"]) ?? {};
    const inputTokens =
      num(usage["input_tokens"]) +
      num(usage["cache_creation_input_tokens"]) +
      num(usage["cache_read_input_tokens"]);
    return { inputTokens, outputTokens: num(usage["output_tokens"]) };
  }

  function handleResultMessage(m: Record<string, unknown>): void {
    const subtype = str(m["subtype"]);
    const turns = num(m["num_turns"]);
    const usage = usageFrom(m);

    if (subtype === "success") {
      const structured = structuredOutputSchema.safeParse(m["structured_output"]);
      const fallbackText = str(m["result"]) ?? "";
      const parsedFallback = !structured.success ? tryParseReportJson(fallbackText) : undefined;
      const report = structured.success ? structured.data.report : (parsedFallback?.report ?? fallbackText);
      const modelCitations = structured.success ? structured.data.citations : parsedFallback?.citations;
      if (!report.trim()) {
        emitFailed("agent_empty_report", "the agent finished without producing a report", "research");
        return;
      }
      emitCompleted({ report, citations: store.finalize(modelCitations), turns, usage });
      return;
    }

    const errors = Array.isArray(m["errors"])
      ? (m["errors"] as unknown[]).filter((e): e is string => typeof e === "string")
      : [];
    // Claude Code error strings can embed API response bodies — same hygiene as the generic path.
    const firstError = errors[0];
    const detail = firstError ? `: ${sanitizeErrorMessage(firstError)}` : "";
    switch (subtype) {
      case "error_max_turns":
        emitFailed("max_turns_exceeded", `research stopped after ${turns} turns${detail}`, "research");
        return;
      case "error_max_budget_usd":
        emitFailed("budget_exceeded", `research stopped: budget exceeded${detail}`, "research");
        return;
      case "error_max_structured_output_retries":
        emitFailed(
          "structured_output_failed",
          `the agent could not produce valid structured output${detail}`,
          "research",
        );
        return;
      case "error_during_execution":
      default:
        emitFailed("agent_execution_failed", `agent execution failed${detail}`, "research");
        return;
    }
  }

  const prompt =
    `Research query: ${request.query}\n\n` +
    `Goal: ${request.goal}\n\n` +
    "Investigate with your tools, then finish with the structured output: the full Markdown " +
    "report in `report` and the sources you actually used in `citations`.";

  function systemPrompt(handlers: ToolHandlers, activeToolNames: ResearchToolName[]): string {
    return buildSystemPrompt({
      goal: request.goal,
      toolNames: activeToolNames,
      hasDocuments: Boolean(handlers.document_read && (request.allowedDocumentIds?.length ?? 0) > 0),
    });
  }

  // ── Claude backend ────────────────────────────────────────────────────────

  async function runClaude(handlers: ToolHandlers, activeToolNames: ResearchToolName[]): Promise<void> {
    const sdkTools = activeToolNames.map((name) =>
      tool(name, TOOL_SPECS[name].description, TOOL_SPECS[name].shape, async (args) =>
        handlers[name]!(args as Record<string, unknown>),
      ),
    );

    const server = createSdkMcpServer({
      name: MCP_SERVER_NAME,
      version: "0.1.0",
      tools: sdkTools,
      timeout: 60_000,
    });

    tmpDir = mkdtempSync(join(tmpdir(), "bluey-agent-"));

    const baseEnv = deps.env ?? process.env;
    const subprocessEnv = buildSubprocessEnv(baseEnv, config);

    const cliPath = resolveClaudeCliPath({
      embeddedClaudePath: deps.embeddedClaudePath,
      env: baseEnv,
    });

    const options: Options = {
      abortController,
      systemPrompt: systemPrompt(handlers, activeToolNames),
      // Remove ALL built-in tools; the agent can only use our MCP tools.
      tools: [],
      allowedTools: activeToolNames.map((name) => `${MCP_TOOL_PREFIX}${name}`),
      disallowedTools: DISALLOWED_BUILTIN_TOOLS,
      permissionMode: "dontAsk",
      mcpServers: { [MCP_SERVER_NAME]: server },
      maxTurns,
      model,
      cwd: tmpDir,
      env: subprocessEnv,
      includePartialMessages: true,
      persistSession: false,
      settingSources: [],
      outputFormat: { type: "json_schema", schema: REPORT_OUTPUT_SCHEMA },
      ...(cliPath ? { pathToClaudeCodeExecutable: cliPath } : {}),
      // Running under Bun (dev and the compiled binary): tell the SDK to use
      // bun as the JS runtime instead of auto-detection.
      ...(typeof process.versions.bun === "string" ? { executable: "bun" as const } : {}),
    };

    const queryFn: QueryFn =
      deps.queryFn ??
      (config.mockMode ? createMockQueryFn({ request, handlers }) : (params) => query(params));

    let sawResult = false;
    for await (const raw of queryFn({ prompt, options })) {
      const m = rec(raw);
      if (!m) continue;
      switch (m["type"]) {
        case "system":
          if (m["subtype"] === "init") {
            const initModel = str(m["model"]) ?? model;
            const toolCount = Array.isArray(m["tools"])
              ? (m["tools"] as unknown[]).length
              : activeToolNames.length;
            progress(`agent session started (model ${initModel}, ${toolCount} tool(s))`);
          }
          break;
        case "stream_event":
          handleStreamEvent(m);
          break;
        case "assistant":
          handleAssistantMessage(m);
          break;
        case "result":
          sawResult = true;
          if (cancelRequested) {
            emitCancelled();
          } else {
            handleResultMessage(m);
          }
          break;
        default:
          break;
      }
    }

    if (!finished) {
      if (cancelRequested) emitCancelled();
      else if (!sawResult)
        emitFailed("agent_no_result", "the agent stream ended without a result message", "research");
    }
  }

  // ── Gemini backend ────────────────────────────────────────────────────────

  async function runGeminiBackend(
    handlers: ToolHandlers,
    activeToolNames: ResearchToolName[],
  ): Promise<void> {
    const generate: GenerateFn =
      deps.generateFn ??
      (config.mockMode
        ? createMockGeminiGenerate({ request, handlers })
        : createGeminiGenerate(config.geminiApiKey ?? ""));
    progress(`agent session started (model ${model}, ${activeToolNames.length} tool(s))`);

    let outcome;
    try {
      outcome = await runGemini({
        model,
        maxTurns,
        systemPrompt: systemPrompt(handlers, activeToolNames),
        prompt,
        handlers,
        activeToolNames,
        signal: abortController.signal,
        generate,
        onTextDelta: (text) => writer.event("research.textDelta", { jobId, text }),
        onToolCall: (toolName, input) => writer.event("research.toolCall", { jobId, tool: toolName, input }),
        onProgress: progress,
      });
    } catch (err) {
      if (cancelRequested || (err instanceof Error && err.name === "AbortError")) {
        emitCancelled();
        return;
      }
      const mapped = mapGeminiError(err);
      if (mapped) {
        emitFailed(mapped.code, mapped.message, mapped.kind);
        return;
      }
      throw err;
    }
    if (cancelRequested) {
      emitCancelled();
      return;
    }

    const structured = tryParseReportJson(outcome.reportJson);
    const report = structured?.report ?? outcome.reportJson;
    if (!report.trim()) {
      emitFailed("agent_empty_report", "the agent finished without producing a report", "research");
      return;
    }
    emitCompleted({
      report,
      citations: store.finalize(structured?.citations),
      turns: outcome.turns,
      usage: outcome.usage,
    });
  }

  // ── Main run ───────────────────────────────────────────────────────────────

  async function run(): Promise<void> {
    writer.event("research.started", { jobId, model });

    const usingInjectedModel =
      config.mockMode || Boolean(config.backend === "gemini" ? deps.generateFn : deps.queryFn);
    if (!usingInjectedModel) {
      const problem =
        checkModelCredentials(config) ??
        checkClaudeCliAvailable(config, {
          variant: deps.buildVariant,
          embeddedClaudePath: deps.embeddedClaudePath,
        });
      if (problem) {
        emitFailed(problem.code, problem.message, "configuration");
        return;
      }
    }

    const handlersOrMissing = buildHandlers();
    if ("missingKey" in handlersOrMissing) {
      emitFailed(
        "missing_api_key",
        `${handlersOrMissing.missingKey} is not set — required by the requested tools`,
        "configuration",
      );
      return;
    }
    const handlers = handlersOrMissing;
    const activeToolNames = request.tools.filter((name) => handlers[name]);

    if (config.backend === "gemini") {
      await runGeminiBackend(handlers, activeToolNames);
    } else {
      await runClaude(handlers, activeToolNames);
    }
  }

  const done = run()
    .catch((err: unknown) => {
      if (cancelRequested || (err instanceof Error && err.name === "AbortError")) {
        emitCancelled();
        return;
      }
      if (err instanceof GeminiRunError) {
        emitFailed(err.code, err.message, err.kind);
        return;
      }
      // Arbitrary SDK/runtime text (status-less ApiErrors embed response
      // bodies, URLs can carry key= values) — never forwarded verbatim.
      const message = err instanceof Error ? err.message : String(err);
      emitFailed("agent_execution_failed", sanitizeErrorMessage(message), "research");
    })
    .finally(() => {
      broker.close();
      if (tmpDir) {
        try {
          rmSync(tmpDir, { recursive: true, force: true });
        } catch {
          // best effort
        }
      }
    });

  return {
    jobId,
    done,
    cancel(reason = "cancel requested"): void {
      if (finished) return;
      cancelRequested = true;
      broker.close(reason);
      abortController.abort();
    },
    handleDocumentResponse(params: DocumentResponseParams): boolean {
      return broker.handleResponse(params);
    },
  };
}
