/**
 * Mock mode (BLUEY_AGENT_MOCK=1): runs a whole research job against a fake
 * model (Claude `query()` or Gemini `generateContentStream`) and fake
 * Exa/Firecrawl clients — no network, no model, no CLI subprocess — so the
 * stdio protocol can be exercised end-to-end from a shell or from tests. The
 * document_read round-trip still goes through the real `document.request` /
 * `document.response` protocol.
 */

import type { Part } from "@google/genai";

import type { QueryFn, ToolHandlers } from "./agent";
import type { GenerateFn, GeminiStreamChunk } from "./gemini";
import type { DeepResearchRequest, ResearchToolName } from "./protocol";
import type { ExaClient } from "./tools/exa";
import type { FirecrawlClient } from "./tools/firecrawl";

export const MOCK_URLS = [
  "https://example.org/bluey/overview",
  "https://example.org/bluey/deep-dive",
] as const;

/** Token usage every mock backend reports for the whole job. */
export const MOCK_USAGE = { inputTokens: 1200, outputTokens: 300 } as const;

export function createMockExaClient(): ExaClient {
  return {
    async search({ query, numResults }) {
      const n = Math.max(1, Math.min(numResults ?? MOCK_URLS.length, MOCK_URLS.length));
      return MOCK_URLS.slice(0, n).map((url, i) => ({
        title: `Mock result ${i + 1} for "${query}"`,
        url,
        snippet: `Mock snippet ${i + 1} about ${query}.`,
        publishedDate: "2026-01-15",
      }));
    },
  };
}

export function createMockFirecrawlClient(): FirecrawlClient {
  return {
    async scrape(url: string) {
      return {
        url,
        title: "Mock scraped page",
        markdown: `# Mock page\n\nContent scraped from ${url}.\n\nKey fact: bluey-agent mock mode is working.`,
        truncated: false,
      };
    },
  };
}

function abortError(): Error {
  const err = new Error("The operation was aborted");
  err.name = "AbortError";
  return err;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function assistantToolUse(name: string, input: Record<string, unknown>): Record<string, unknown> {
  return {
    type: "assistant",
    parent_tool_use_id: null,
    message: {
      role: "assistant",
      content: [{ type: "tool_use", id: `toolu_mock_${name}`, name: `mcp__bluey__${name}`, input }],
    },
  };
}

function chunked(text: string, parts: number): string[] {
  const size = Math.ceil(text.length / parts);
  const out: string[] = [];
  for (let i = 0; i < text.length; i += size) out.push(text.slice(i, i + size));
  return out;
}

function mockReport(request: DeepResearchRequest, backend: string): string {
  return (
    `## Mock research report\n\n` +
    `**Query:** ${request.query}\n\n**Goal:** ${request.goal}\n\n` +
    `- Finding backed by a mock source ([Mock result 1](${MOCK_URLS[0]})).\n` +
    `- Inference: this run used BLUEY_AGENT_MOCK=1 (${backend}) — no network, no model, no CLI subprocess.\n`
  );
}

/** Scripted tool order shared by both mocks; `document_read` only with an allowed id. */
function scriptedToolCalls(
  request: DeepResearchRequest,
  handlers: ToolHandlers,
): Array<{ name: ResearchToolName; input: Record<string, unknown> }> {
  const plan: Array<{ name: ResearchToolName; input: Record<string, unknown> }> = [];
  if (handlers.exa_search) plan.push({ name: "exa_search", input: { query: request.query, numResults: 2 } });
  if (handlers.firecrawl_scrape) plan.push({ name: "firecrawl_scrape", input: { url: MOCK_URLS[0] } });
  const docId = request.allowedDocumentIds?.[0];
  if (handlers.document_read && docId) plan.push({ name: "document_read", input: { documentId: docId } });
  return plan;
}

export interface MockQueryContext {
  request: DeepResearchRequest;
  handlers: ToolHandlers;
  /** Delay between scripted steps (ms). Kept tiny in tests. */
  stepMs?: number;
}

/** Fake Claude Agent SDK `query()`: emits SDK-shaped messages and runs the handlers itself. */
export function createMockQueryFn(ctx: MockQueryContext): QueryFn {
  const { request, handlers } = ctx;
  const stepMs = ctx.stepMs ?? 25;

  return ({ options }) => {
    const signal = options.abortController?.signal;

    async function step(): Promise<void> {
      await sleep(stepMs);
      if (signal?.aborted) throw abortError();
    }

    async function* generate(): AsyncGenerator<unknown, void, undefined> {
      yield {
        type: "system",
        subtype: "init",
        model: "mock-claude",
        tools: Object.keys(handlers).map((n) => `mcp__bluey__${n}`),
      };

      let toolCalls = 0;
      for (const call of scriptedToolCalls(request, handlers)) {
        await step();
        yield assistantToolUse(call.name, call.input);
        await handlers[call.name]!(call.input);
        toolCalls += 1;
      }

      const report = mockReport(request, "claude");
      for (const part of chunked(report, 3)) {
        await step();
        yield {
          type: "stream_event",
          parent_tool_use_id: null,
          event: { type: "content_block_delta", delta: { type: "text_delta", text: part } },
        };
      }

      await step();
      yield {
        type: "result",
        subtype: "success",
        is_error: false,
        num_turns: toolCalls + 2,
        duration_ms: (toolCalls + 5) * stepMs,
        result: report,
        structured_output: {
          report,
          citations: [{ title: "Mock result 1", url: MOCK_URLS[0], snippet: "Mock snippet 1." }],
        },
        usage: { input_tokens: MOCK_USAGE.inputTokens, output_tokens: MOCK_USAGE.outputTokens },
      };
    }

    return generate();
  };
}

/**
 * Fake `generateContentStream`: one scripted function call per turn (the real
 * loop executes the handlers and feeds back `functionResponse`s), then a
 * narrative text turn, then the schema-constrained JSON report when the final
 * turn asks for `responseJsonSchema`.
 */
export function createMockGeminiGenerate(ctx: MockQueryContext): GenerateFn {
  const { request, handlers } = ctx;
  const stepMs = ctx.stepMs ?? 25;
  const plan = scriptedToolCalls(request, handlers);
  let turn = 0;

  return async ({ config }) => {
    const signal = config.abortSignal;
    const report = mockReport(request, "gemini");

    async function step(): Promise<void> {
      await sleep(stepMs);
      if (signal?.aborted) throw abortError();
    }

    const textChunk = (text: string, finishReason?: string): GeminiStreamChunk => ({
      candidates: [
        { content: { role: "model", parts: [{ text }] }, ...(finishReason ? { finishReason } : {}) },
      ],
    });

    async function* generate(): AsyncGenerator<GeminiStreamChunk, void, undefined> {
      await step();
      if (config.responseJsonSchema) {
        const json = JSON.stringify({
          report,
          citations: [{ title: "Mock result 1", url: MOCK_URLS[0], snippet: "Mock snippet 1." }],
        });
        yield {
          candidates: [{ content: { role: "model", parts: [{ text: json }] }, finishReason: "STOP" }],
          usageMetadata: {
            promptTokenCount: MOCK_USAGE.inputTokens,
            candidatesTokenCount: MOCK_USAGE.outputTokens,
          },
        };
        return;
      }
      const call = plan[turn];
      turn += 1;
      if (call) {
        const part: Part = {
          functionCall: { id: `call_mock_${turn}`, name: call.name, args: call.input },
          thoughtSignature: "bW9jay1zaWduYXR1cmU=",
        };
        yield { candidates: [{ content: { role: "model", parts: [part] }, finishReason: "STOP" }] };
        return;
      }
      const pieces = chunked(report, 3);
      for (let i = 0; i < pieces.length; i += 1) {
        if (i > 0) await step();
        yield textChunk(pieces[i]!, i === pieces.length - 1 ? "STOP" : undefined);
      }
    }

    return generate();
  };
}
