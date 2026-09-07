/**
 * Mock mode (BLUEY_AGENT_MOCK=1): runs a whole research job against a fake
 * `query()` and fake Exa/Firecrawl clients — no network, no model, no CLI
 * subprocess — so the stdio protocol can be exercised end-to-end from a shell
 * or from tests. The document_read round-trip still goes through the real
 * `document.request` / `document.response` protocol.
 */

import type { QueryFn, ToolHandlers } from "./agent";
import type { DeepResearchRequest } from "./protocol";
import type { ExaClient } from "./tools/exa";
import type { FirecrawlClient } from "./tools/firecrawl";

export const MOCK_URLS = [
  "https://example.org/bluey/overview",
  "https://example.org/bluey/deep-dive",
] as const;

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

export interface MockQueryContext {
  request: DeepResearchRequest;
  handlers: ToolHandlers;
  /** Delay between scripted steps (ms). Kept tiny in tests. */
  stepMs?: number;
}

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

      if (handlers.exa_search) {
        await step();
        yield assistantToolUse("exa_search", { query: request.query, numResults: 2 });
        await handlers.exa_search({ query: request.query, numResults: 2 });
        toolCalls += 1;
      }

      if (handlers.firecrawl_scrape) {
        await step();
        yield assistantToolUse("firecrawl_scrape", { url: MOCK_URLS[0] });
        await handlers.firecrawl_scrape({ url: MOCK_URLS[0] });
        toolCalls += 1;
      }

      const docId = request.allowedDocumentIds?.[0];
      if (handlers.document_read && docId) {
        await step();
        yield assistantToolUse("document_read", { documentId: docId });
        // Real document.request/document.response round-trip (10 s timeout if
        // the shell/Rust side never answers; the error becomes a tool result).
        await handlers.document_read({ documentId: docId });
        toolCalls += 1;
      }

      const report =
        `## Mock research report\n\n` +
        `**Query:** ${request.query}\n\n**Goal:** ${request.goal}\n\n` +
        `- Finding backed by a mock source ([Mock result 1](${MOCK_URLS[0]})).\n` +
        `- Inference: this run used BLUEY_AGENT_MOCK=1 — no network, no model, no CLI subprocess.\n`;

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
        usage: { input_tokens: 1200, output_tokens: 300 },
      };
    }

    return generate();
  };
}
