/**
 * Gemini backend: the function-calling loop over an injected
 * `generateContentStream` — tool round-trips with echoed call ids, the final
 * schema-constrained report turn, cancellation mid-loop, the turn budget,
 * refusals and API error mapping. Network-free.
 */

import { PassThrough } from "node:stream";
import { describe, expect, it } from "vitest";

import type { GenerateFn, GenerateParams, GeminiStreamChunk } from "../../sidecars/agent/src/gemini";
import {
  functionDeclarations,
  GEMINI_REPORT_SCHEMA,
  mapGeminiError,
  runGemini,
} from "../../sidecars/agent/src/gemini";
import { startSidecar, type StartSidecarOptions } from "../../sidecars/agent/src/main";
import { toolParametersJsonSchema } from "../../sidecars/agent/src/tool-specs";

type Frame = Record<string, unknown>;

function makeHarness(options: Omit<StartSidecarOptions, "input" | "output">) {
  const input = new PassThrough();
  const frames: Frame[] = [];
  const waiters: Array<{ pred: (f: Frame) => boolean; resolve: (f: Frame) => void }> = [];
  const output = {
    write(chunk: string) {
      for (const line of chunk.split("\n")) {
        if (!line.trim()) continue;
        const frame = JSON.parse(line) as Frame;
        frames.push(frame);
        for (let i = waiters.length - 1; i >= 0; i -= 1) {
          const waiter = waiters[i]!;
          if (waiter.pred(frame)) {
            waiters.splice(i, 1);
            waiter.resolve(frame);
          }
        }
      }
      return true;
    },
  };
  const done = startSidecar({ ...options, input, output, installSignalHandlers: false });
  return {
    frames,
    done,
    send: (frame: Record<string, unknown>) => input.write(`${JSON.stringify(frame)}\n`),
    waitFor: (pred: (f: Frame) => boolean, label: string) => {
      const existing = frames.find(pred);
      if (existing) return Promise.resolve(existing);
      return new Promise<Frame>((resolve, reject) => {
        const timer = setTimeout(
          () => reject(new Error(`timed out waiting for ${label}; saw: ${JSON.stringify(frames, null, 2)}`)),
          5_000,
        );
        waiters.push({
          pred,
          resolve: (frame) => {
            clearTimeout(timer);
            resolve(frame);
          },
        });
      });
    },
    eventsNamed: (name: string) => frames.filter((f) => f["event"] === name),
  };
}

const isEvent = (name: string) => (frame: Frame) => frame["event"] === name;

async function* chunks(...items: GeminiStreamChunk[]): AsyncGenerator<GeminiStreamChunk> {
  for (const item of items) yield item;
}

const textChunk = (
  text: string,
  finishReason?: string,
  usage?: GeminiStreamChunk["usageMetadata"],
): GeminiStreamChunk => ({
  candidates: [{ content: { role: "model", parts: [{ text }] }, ...(finishReason ? { finishReason } : {}) }],
  ...(usage ? { usageMetadata: usage } : {}),
});

const callChunk = (id: string, name: string, args: Record<string, unknown>): GeminiStreamChunk => ({
  candidates: [
    {
      content: { role: "model", parts: [{ functionCall: { id, name, args }, thoughtSignature: "c2ln" }] },
      finishReason: "STOP",
    },
  ],
  usageMetadata: { promptTokenCount: 100, candidatesTokenCount: 10, thoughtsTokenCount: 5 },
});

const report = "## Report\n\nBun sidecars are great ([Exa 1](https://example.org/a)).";
const reportJson = JSON.stringify({
  report,
  citations: [
    { title: "Exa 1", url: "https://example.org/a", snippet: "s" },
    { title: "Invented", url: "https://invented.example/nope" },
  ],
});

/** Scripted model: exa_search → text → JSON report; records every request (contents snapshotted). */
function scriptedGenerate(seen: GenerateParams[]): GenerateFn {
  let turn = 0;
  return async (params) => {
    seen.push({ ...params, contents: [...params.contents] });
    turn += 1;
    if (params.config.responseJsonSchema) {
      return chunks(textChunk(reportJson, "STOP", { promptTokenCount: 500, candidatesTokenCount: 200 }));
    }
    if (turn === 1)
      return chunks(callChunk("call-1", "exa_search", { query: "bun sidecars", numResults: 2 }));
    return chunks(
      textChunk("I found ", undefined),
      textChunk("enough.", "STOP", { promptTokenCount: 300, candidatesTokenCount: 20 }),
    );
  };
}

const exaClient = {
  async search() {
    return [
      { title: "Exa 1", url: "https://example.org/a", snippet: "snippet a" },
      { title: "Exa 2", url: "https://example.org/b", snippet: "snippet b" },
    ];
  },
};

const runParams = {
  jobId: "job-g",
  query: "bun sidecars",
  goal: "explain bun sidecars",
  tools: ["exa_search"],
};

describe("Gemini backend (injected generateContentStream)", () => {
  it("runs the tool loop, echoes call ids, streams text and finishes with the structured report", async () => {
    const seen: GenerateParams[] = [];
    const harness = makeHarness({
      env: { GEMINI_API_KEY: "AIza-test" },
      deps: { generateFn: scriptedGenerate(seen), exaClient },
    });
    harness.send({ id: 1, method: "research.run", params: runParams });

    const completed = await harness.waitFor(isEvent("research.completed"), "completed");
    expect(await harness.done).toBe(0);

    const names = harness.frames.filter((f) => f["event"]).map((f) => f["event"]);
    expect(names[0]).toBe("research.started");
    expect(names[names.length - 1]).toBe("research.completed");
    expect((harness.eventsNamed("research.started")[0]!["data"] as Frame)["model"]).toBe("gemini-3.8-flash");

    const toolCall = harness.eventsNamed("research.toolCall")[0]!["data"] as Frame;
    expect(toolCall["tool"]).toBe("exa_search");
    expect(toolCall["input"]).toEqual({ query: "bun sidecars", numResults: 2 });

    const deltas = harness.eventsNamed("research.textDelta").map((f) => (f["data"] as Frame)["text"]);
    expect(deltas).toEqual(["I found ", "enough."]);

    const data = completed["data"] as Frame;
    expect(data["report"]).toBe(report);
    expect(data["turns"]).toBe(3);
    expect(data["usage"]).toEqual({ inputTokens: 900, outputTokens: 235 });
    // Model citations validated against tool-observed URLs; the invented one is dropped.
    const citations = data["citations"] as Array<{ url: string }>;
    expect(citations.map((c) => c.url)).toEqual(["https://example.org/a", "https://example.org/b"]);

    // Request shapes: tool turns carry declarations + low thinking; the report turn has no tools.
    expect(seen).toHaveLength(3);
    const first = seen[0]!;
    expect(first.model).toBe("gemini-3.8-flash");
    expect(first.config.systemInstruction).toContain("research analyst");
    expect(first.config.thinkingConfig).toEqual({ thinkingLevel: "LOW" });
    const declarations = (first.config.tools as Array<{ functionDeclarations: Array<{ name: string }> }>)[0]!
      .functionDeclarations;
    expect(declarations.map((d) => d.name)).toEqual(["exa_search"]);
    expect(first.config.abortSignal).toBeInstanceOf(AbortSignal);
    expect(first.config.httpOptions).toEqual({ timeout: 60_000, retryOptions: { attempts: 3 } });

    const second = seen[1]!;
    expect(second.contents).toHaveLength(3);
    expect(second.contents[1]!.role).toBe("model");
    expect(second.contents[1]!.parts?.[0]?.thoughtSignature).toBe("c2ln");
    const response = second.contents[2]!;
    expect(response.role).toBe("user");
    expect(response.parts?.[0]?.functionResponse?.id).toBe("call-1");
    expect(response.parts?.[0]?.functionResponse?.name).toBe("exa_search");
    expect(JSON.stringify(response.parts?.[0]?.functionResponse?.response)).toContain(
      "https://example.org/a",
    );

    const last = seen[2]!;
    expect(last.config.tools).toBeUndefined();
    expect(last.config.responseMimeType).toBe("application/json");
    expect(last.config.responseJsonSchema).toBe(GEMINI_REPORT_SCHEMA);
    expect(last.config.thinkingConfig).toBeUndefined();
    expect(JSON.stringify(harness.frames)).not.toContain("AIza-test");
  });

  it("maps research.cancel mid-loop onto cancelled", async () => {
    // Turn 1 completes with a tool call (so `research.toolCall` is observable);
    // turn 2 hangs until the job's AbortSignal fires, like a stalled stream.
    let turn = 0;
    const hanging: GenerateFn = async ({ config }) => {
      turn += 1;
      if (turn === 1) return chunks(callChunk("c", "exa_search", { query: "x" }));
      const stalled: AsyncIterable<GeminiStreamChunk> = {
        [Symbol.asyncIterator]: () => ({
          next: () =>
            new Promise<IteratorResult<GeminiStreamChunk>>((_resolve, reject) => {
              const signal = config.abortSignal!;
              const abort = () => {
                const err = new Error("aborted");
                err.name = "AbortError";
                reject(err);
              };
              if (signal.aborted) abort();
              else signal.addEventListener("abort", abort, { once: true });
            }),
        }),
      };
      return stalled;
    };
    const harness = makeHarness({ env: { GEMINI_API_KEY: "k" }, deps: { generateFn: hanging, exaClient } });
    harness.send({ id: 1, method: "research.run", params: runParams });
    await harness.waitFor(isEvent("research.toolCall"), "first tool call");
    harness.send({ id: 2, method: "research.cancel", params: { jobId: "job-g" } });
    const failed = await harness.waitFor(isEvent("research.failed"), "failed");
    expect((failed["data"] as Frame)["error"]).toMatchObject({ code: "cancelled", kind: "cancelled" });
    expect(await harness.done).toBe(0);
  });

  it("stops with max_turns_exceeded when the model keeps calling tools", async () => {
    let n = 0;
    const alwaysCalling: GenerateFn = async () => {
      n += 1;
      return chunks(callChunk(`call-${n}`, "exa_search", { query: `q${n}` }));
    };
    const harness = makeHarness({
      env: { GEMINI_API_KEY: "k" },
      deps: { generateFn: alwaysCalling, exaClient },
    });
    harness.send({ id: 1, method: "research.run", params: { ...runParams, maxTurns: 3 } });
    const failed = await harness.waitFor(isEvent("research.failed"), "failed");
    const error = (failed["data"] as Frame)["error"] as Frame;
    expect(error["code"]).toBe("max_turns_exceeded");
    expect(error["kind"]).toBe("research");
    expect(harness.eventsNamed("research.toolCall")).toHaveLength(2);
    expect(await harness.done).toBe(0);
  });

  it("reports refusals as blocked", async () => {
    const refusing: GenerateFn = async () => chunks({ promptFeedback: { blockReason: "SAFETY" } });
    const harness = makeHarness({ env: { GEMINI_API_KEY: "k" }, deps: { generateFn: refusing, exaClient } });
    harness.send({ id: 1, method: "research.run", params: runParams });
    const failed = await harness.waitFor(isEvent("research.failed"), "failed");
    expect((failed["data"] as Frame)["error"]).toMatchObject({ code: "blocked", kind: "research" });
    expect(await harness.done).toBe(0);
  });

  it("maps API errors onto invalid_api_key / rate_limited without echoing bodies", async () => {
    const apiError = (status: number, message: string) => Object.assign(new Error(message), { status });
    expect(mapGeminiError(apiError(400, "API key not valid. Please pass a valid API key."))).toMatchObject({
      code: "invalid_api_key",
      kind: "configuration",
    });
    expect(mapGeminiError(apiError(403, "PERMISSION_DENIED"))).toMatchObject({ code: "invalid_api_key" });
    expect(mapGeminiError(apiError(429, "quota exceeded prompt-echo"))).toMatchObject({
      code: "rate_limited",
    });
    expect(mapGeminiError(apiError(429, "x"))?.message).not.toContain("prompt-echo");
    expect(mapGeminiError(apiError(503, "unavailable"))).toMatchObject({ code: "agent_execution_failed" });
    expect(mapGeminiError(new Error("plain"))).toBeUndefined();

    const rateLimited: GenerateFn = async () => {
      throw apiError(429, "RESOURCE_EXHAUSTED body with prompt text");
    };
    const harness = makeHarness({
      env: { GEMINI_API_KEY: "k" },
      deps: { generateFn: rateLimited, exaClient },
    });
    harness.send({ id: 1, method: "research.run", params: runParams });
    const failed = await harness.waitFor(isEvent("research.failed"), "failed");
    const error = (failed["data"] as Frame)["error"] as Frame;
    expect(error["code"]).toBe("rate_limited");
    expect(error["message"]).not.toContain("prompt text");
    expect(await harness.done).toBe(0);
  });

  it("derives function declarations from the shared zod tool shapes", () => {
    const declarations = functionDeclarations(["exa_search", "firecrawl_scrape", "document_read"]);
    expect(declarations.map((d) => d.name)).toEqual(["exa_search", "firecrawl_scrape", "document_read"]);
    const exa = declarations[0]!.parametersJsonSchema;
    expect(exa["type"]).toBe("object");
    expect(exa["$schema"]).toBeUndefined();
    expect((exa["properties"] as Record<string, unknown>)["query"]).toMatchObject({ type: "string" });
    expect(exa["required"]).toEqual(["query"]);
    expect(toolParametersJsonSchema("document_read")["required"]).toEqual(["documentId"]);
  });

  it("runGemini falls through to the report turn when the model answers without tools", async () => {
    const seen: GenerateParams[] = [];
    const generate: GenerateFn = async (params) => {
      seen.push(params);
      if (params.config.responseJsonSchema) return chunks(textChunk(reportJson, "STOP"));
      return chunks(textChunk("no tools needed", "STOP"));
    };
    const result = await runGemini({
      model: "gemini-3.8-flash",
      maxTurns: 2,
      systemPrompt: "sys",
      prompt: "prompt",
      handlers: {},
      activeToolNames: [],
      signal: new AbortController().signal,
      generate,
      onTextDelta: () => {},
      onToolCall: () => {},
      onProgress: () => {},
    });
    expect(result.turns).toBe(2);
    expect(JSON.parse(result.reportJson)).toMatchObject({ report });
    expect(seen[0]!.config.tools).toBeUndefined();
  });
});
