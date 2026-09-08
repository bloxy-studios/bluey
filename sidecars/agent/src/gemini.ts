/**
 * Gemini backend for the research job: a function-calling loop over
 * `generateContentStream` (`@google/genai`, Google AI Studio key) that reuses
 * the same tool handlers, citation store and document broker as the Claude
 * backend, then one final schema-constrained "write the report" turn.
 *
 * Wire rules (ai.google.dev, verified 2026-09-08):
 *  - push the model's `content` back UNCHANGED between turns (it carries
 *    `thoughtSignature`s), then one `user` turn holding every `functionResponse`
 *    whose `id` equals the matching `functionCall.id`;
 *  - Gemini 3.x: `thinkingLevel` instead of budgets, no sampling params;
 *  - structured output = `responseMimeType: application/json` +
 *    `responseJsonSchema`; the tool loop runs in text mode and the final report
 *    turn runs without tools (the robust pattern for 3.x).
 */

import {
  ApiError,
  GoogleGenAI,
  ThinkingLevel,
  type Content,
  type GenerateContentConfig,
  type Part,
} from "@google/genai";

import type { ToolHandlers } from "./agent";
import type { ResearchToolName } from "./protocol";
import { TOOL_SPECS, toolParametersJsonSchema } from "./tool-specs";

/** Structural subset of `GenerateContentResponse` the loop reads (mock-friendly). */
export interface GeminiStreamChunk {
  candidates?: Array<{
    content?: { role?: string; parts?: Part[] };
    finishReason?: string;
  }>;
  usageMetadata?: {
    promptTokenCount?: number;
    candidatesTokenCount?: number;
    thoughtsTokenCount?: number;
  };
  promptFeedback?: { blockReason?: string };
}

export interface GenerateParams {
  model: string;
  contents: Content[];
  config: GenerateContentConfig;
}

/** Anything that behaves like `ai.models.generateContentStream` (tests and mock mode inject one). */
export type GenerateFn = (params: GenerateParams) => Promise<AsyncIterable<GeminiStreamChunk>>;

export interface GeminiRunOptions {
  model: string;
  maxTurns: number;
  systemPrompt: string;
  prompt: string;
  handlers: ToolHandlers;
  activeToolNames: ResearchToolName[];
  signal: AbortSignal;
  generate: GenerateFn;
  onTextDelta(text: string): void;
  onToolCall(tool: string, input: Record<string, unknown>): void;
  onProgress(message: string): void;
}

export interface GeminiRunResult {
  /** Raw text of the final structured turn (JSON per `REPORT_OUTPUT_SCHEMA`). */
  reportJson: string;
  turns: number;
  usage: { inputTokens: number; outputTokens: number };
}

/** Failure the loop can report without throwing a generic error. */
export class GeminiRunError extends Error {
  constructor(
    public readonly code:
      | "max_turns_exceeded"
      | "blocked"
      | "rate_limited"
      | "invalid_api_key"
      | "agent_empty_report"
      | "agent_execution_failed",
    message: string,
    public readonly kind: "research" | "configuration" = "research",
  ) {
    super(message);
    this.name = "GeminiRunError";
  }
}

const REQUEST_TIMEOUT_MS = 60_000;
const RETRY_ATTEMPTS = 3;

/** Structured report schema handed to the final turn (same shape as the Claude backend's). */
export const GEMINI_REPORT_SCHEMA: Record<string, unknown> = {
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

/** Default `generate` — the real SDK. Never reads the key from anywhere but `apiKey`. */
export function createGeminiGenerate(apiKey: string): GenerateFn {
  const ai = new GoogleGenAI({ apiKey });
  return (params) => ai.models.generateContentStream(params);
}

/** Function declarations for the requested tools (Gemini `tools[0].functionDeclarations`). */
export function functionDeclarations(names: ResearchToolName[]): Array<{
  name: string;
  description: string;
  parametersJsonSchema: Record<string, unknown>;
}> {
  return names.map((name) => ({
    name,
    description: TOOL_SPECS[name].description,
    parametersJsonSchema: toolParametersJsonSchema(name),
  }));
}

/** Text of the turn's parts (thought summaries excluded). */
function visibleText(parts: Part[]): string {
  return parts
    .filter((part) => !part.thought && typeof part.text === "string")
    .map((part) => part.text ?? "")
    .join("");
}

/** finishReason values that mean the answer was refused rather than finished. */
function isRefusal(finishReason: string | undefined): boolean {
  if (!finishReason) return false;
  return !["STOP", "MAX_TOKENS", "FINISH_REASON_UNSPECIFIED"].includes(finishReason.toUpperCase());
}

/**
 * Accumulate a streamed turn: forward visible text deltas, keep every part so
 * the model turn can be resent unchanged (signatures intact), and capture usage.
 */
async function collectTurn(
  stream: AsyncIterable<GeminiStreamChunk>,
  signal: AbortSignal,
  onTextDelta?: (text: string) => void,
): Promise<{
  parts: Part[];
  finishReason?: string;
  usage: { input: number; output: number };
}> {
  const parts: Part[] = [];
  let finishReason: string | undefined;
  let usage = { input: 0, output: 0 };
  for await (const chunk of stream) {
    if (signal.aborted) throw abortError();
    const blockReason = chunk.promptFeedback?.blockReason;
    if (blockReason) {
      throw new GeminiRunError("blocked", `the model refused the prompt (${blockReason.toLowerCase()})`);
    }
    const candidate = chunk.candidates?.[0];
    for (const part of candidate?.content?.parts ?? []) {
      if (!part.thought && typeof part.text === "string" && part.text && onTextDelta) {
        onTextDelta(part.text);
      }
      parts.push(part);
    }
    if (candidate?.finishReason) finishReason = candidate.finishReason;
    const meta = chunk.usageMetadata;
    if (meta) {
      usage = {
        input: meta.promptTokenCount ?? usage.input,
        output: (meta.candidatesTokenCount ?? 0) + (meta.thoughtsTokenCount ?? 0),
      };
    }
  }
  if (isRefusal(finishReason)) {
    throw new GeminiRunError(
      "blocked",
      `the model refused to answer (${(finishReason ?? "other").toLowerCase()})`,
    );
  }
  return { parts, finishReason, usage };
}

function abortError(): Error {
  const err = new Error("The operation was aborted");
  err.name = "AbortError";
  return err;
}

/** Map SDK/API failures onto protocol codes without echoing response bodies. */
export function mapGeminiError(err: unknown): GeminiRunError | undefined {
  if (err instanceof GeminiRunError) return err;
  const status =
    err instanceof ApiError
      ? err.status
      : typeof (err as { status?: unknown })?.status === "number"
        ? ((err as { status: number }).status as number)
        : undefined;
  if (status === undefined) return undefined;
  const message = err instanceof Error ? err.message : "";
  if (status === 400 && /API_KEY_INVALID|API key not valid/i.test(message)) {
    return new GeminiRunError(
      "invalid_api_key",
      "GEMINI_API_KEY was rejected by Google AI Studio — create a new key at aistudio.google.com/apikey",
      "configuration",
    );
  }
  if (status === 401 || status === 403) {
    return new GeminiRunError(
      "invalid_api_key",
      `GEMINI_API_KEY is not accepted by the Gemini API (HTTP ${status})`,
      "configuration",
    );
  }
  if (status === 429) {
    return new GeminiRunError(
      "rate_limited",
      "the Gemini API rate-limited the research job (HTTP 429) — wait a moment or check the quota in AI Studio",
    );
  }
  return new GeminiRunError("agent_execution_failed", `the Gemini API request failed (HTTP ${status})`);
}

/**
 * Run the tool loop + final report turn. Throws `GeminiRunError` for protocol
 * failures and rethrows abort errors untouched (the caller maps cancellation).
 */
export async function runGemini(options: GeminiRunOptions): Promise<GeminiRunResult> {
  const { model, maxTurns, handlers, activeToolNames, signal, generate } = options;
  const declarations = functionDeclarations(activeToolNames);
  const baseConfig: GenerateContentConfig = {
    systemInstruction: options.systemPrompt,
    abortSignal: signal,
    httpOptions: { timeout: REQUEST_TIMEOUT_MS, retryOptions: { attempts: RETRY_ATTEMPTS } },
  };
  const contents: Content[] = [{ role: "user", parts: [{ text: options.prompt }] }];
  const usage = { inputTokens: 0, outputTokens: 0 };
  let turns = 0;

  const runTurn = async (config: GenerateContentConfig, streamText: boolean) => {
    if (signal.aborted) throw abortError();
    turns += 1;
    const stream = await generate({ model, contents, config });
    const turn = await collectTurn(stream, signal, streamText ? options.onTextDelta : undefined);
    usage.inputTokens += turn.usage.input;
    usage.outputTokens += turn.usage.output;
    return turn;
  };

  // ── Tool loop (text mode) ──────────────────────────────────────────────────
  let toolBudgetExhausted = false;
  for (;;) {
    if (turns >= maxTurns - 1) {
      // Keep one turn for the report.
      toolBudgetExhausted = true;
      break;
    }
    const turn = await runTurn(
      {
        ...baseConfig,
        tools: declarations.length ? [{ functionDeclarations: declarations }] : undefined,
        thinkingConfig: { thinkingLevel: ThinkingLevel.LOW },
      },
      true,
    );
    const calls = turn.parts.filter((part) => part.functionCall);
    // The model turn goes back verbatim (thought signatures included).
    contents.push({ role: "model", parts: turn.parts });
    if (calls.length === 0) break;

    const responses: Part[] = [];
    for (const part of calls) {
      const call = part.functionCall!;
      const name = call.name ?? "";
      const args = call.args ?? {};
      options.onToolCall(name, args);
      const handler = (handlers as Record<string, ToolHandlers[ResearchToolName]>)[name];
      let response: Record<string, unknown>;
      if (!handler) {
        response = { error: `unknown tool "${name}"` };
      } else {
        const result = await handler(args);
        const text = result.content.map((c) => c.text).join("\n");
        response = result.isError ? { error: text } : { result: text };
      }
      responses.push({ functionResponse: { id: call.id, name, response } });
    }
    contents.push({ role: "user", parts: responses });
  }
  if (toolBudgetExhausted) {
    // Only fatal when the model still wanted tools; otherwise the last turn
    // already ended the investigation and the report turn below is turn `maxTurns`.
    const last = contents[contents.length - 1];
    if (last?.role === "user" && last.parts?.some((p) => p.functionResponse)) {
      throw new GeminiRunError(
        "max_turns_exceeded",
        `research stopped after ${turns} turns without a final answer`,
      );
    }
  }

  // ── Final report turn (structured, no tools) ───────────────────────────────
  options.onProgress("writing the report");
  contents.push({
    role: "user",
    parts: [
      {
        text:
          "Write the final research report now. Return JSON with `report` (the full Markdown " +
          "report) and `citations` (only URLs that appeared in tool results).",
      },
    ],
  });
  const final = await runTurn(
    {
      ...baseConfig,
      responseMimeType: "application/json",
      responseJsonSchema: GEMINI_REPORT_SCHEMA,
    },
    false,
  );
  const reportJson = visibleText(final.parts).trim();
  if (!reportJson) {
    throw new GeminiRunError("agent_empty_report", "the model returned an empty report");
  }
  return { reportJson, turns, usage };
}
