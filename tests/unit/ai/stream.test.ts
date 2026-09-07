import {
  CodeFenceBuffer,
  extractPartialStringField,
  streamRequest,
  visibleWithHeldFences,
} from "@/ai/stream";
import type { AIChunk, AIRequest, ModelSelection } from "@/lib/types";

describe("CodeFenceBuffer", () => {
  it("withholds an unterminated fenced block and releases it after the closing fence", () => {
    const buffer = new CodeFenceBuffer();
    expect(buffer.push("Here is the fix:\n")).toBe("Here is the fix:\n");
    expect(buffer.push("```ts\nconst a = 1;")).toBe("Here is the fix:\n");
    expect(buffer.push("\nconst b = 2;")).toBe("Here is the fix:\n");
    const visible = buffer.push("\n```\nDone.");
    expect(visible).toContain("```ts\nconst a = 1;\nconst b = 2;\n```");
    expect(visible).toContain("Done.");
  });

  it("withholds a trailing partial fence marker", () => {
    const buffer = new CodeFenceBuffer();
    expect(buffer.push("text\n`")).toBe("text\n");
    expect(buffer.push("`")).toBe("text\n");
    expect(buffer.push("`js\ncode")).toBe("text\n"); // now a real open fence
  });

  it("flush returns everything, including an unterminated fence", () => {
    const buffer = new CodeFenceBuffer();
    buffer.push("a\n```py\nprint(1)");
    expect(buffer.flush()).toBe("a\n```py\nprint(1)");
  });

  it("handles multiple fenced blocks", () => {
    const text = "one\n```a\nx\n```\ntwo\n```b\ny\n```\nthree";
    expect(visibleWithHeldFences(text)).toBe(text);
    expect(visibleWithHeldFences("one\n```a\nx\n```\ntwo\n```b\ny")).toBe("one\n```a\nx\n```\ntwo\n");
  });
});

describe("extractPartialStringField", () => {
  it("returns null before the field starts", () => {
    expect(extractPartialStringField('{"responseType":"ans')).toBeNull();
  });

  it("extracts a growing content value", () => {
    expect(extractPartialStringField('{"responseType":"answer","content":"Hel')).toBe("Hel");
    expect(extractPartialStringField('{"responseType":"answer","content":"Hello"}')).toBe("Hello");
  });

  it("unescapes JSON escapes including newlines and unicode", () => {
    expect(extractPartialStringField('{"content":"line1\\nline2"')).toBe("line1\nline2");
    expect(extractPartialStringField('{"content":"quote: \\" end"')).toBe('quote: " end');
    expect(extractPartialStringField('{"content":"snow \\u2603"')).toBe("snow ☃");
  });

  it("waits on an escape split across deltas", () => {
    expect(extractPartialStringField('{"content":"abc\\')).toBe("abc");
    expect(extractPartialStringField('{"content":"abc\\u26')).toBe("abc");
  });
});

const SELECTION: ModelSelection = {
  providerId: "mock",
  providerKind: "mock",
  model: "mock-1",
  role: "default",
  reason: "test",
};

function makeRequest(): AIRequest {
  return {
    requestId: "req_stream_test",
    generation: 1,
    task: "answer",
    latencyBudget: "fast",
    reasoning: "none",
    visionRequired: false,
    contextTokens: 100,
    messages: [{ role: "user", content: [{ type: "text", text: "hi" }] }],
    createdAt: "2026-09-07T09:00:00.000Z",
  };
}

function scriptedApi(chunks: AIChunk[]) {
  const cancelled: string[] = [];
  return {
    cancelled,
    api: {
      ai: {
        stream: async (_request: AIRequest, onChunk: (chunk: AIChunk) => void) => {
          for (const chunk of chunks) onChunk(chunk);
        },
        cancel: async ({ requestId }: { requestId: string }) => {
          cancelled.push(requestId);
          return true;
        },
      },
    },
  };
}

describe("streamRequest", () => {
  it("accumulates deltas and resolves with timing + usage", async () => {
    const request = makeRequest();
    const { api } = scriptedApi([
      { type: "started", requestId: request.requestId, selection: SELECTION },
      { type: "delta", requestId: request.requestId, text: "Hello " },
      { type: "delta", requestId: request.requestId, text: "world" },
      { type: "usage", requestId: request.requestId, inputTokens: 42, outputTokens: 7 },
      { type: "completed", requestId: request.requestId, finishReason: "stop", totalMs: 900, timeToFirstTokenMs: 150 },
    ]);
    const deltas: string[] = [];
    const outcome = await streamRequest(request, { onDelta: (d) => deltas.push(d) }, api).done;
    expect(outcome.text).toBe("Hello world");
    expect(deltas).toEqual(["Hello ", "world"]);
    expect(outcome.finishReason).toBe("stop");
    expect(outcome.selection).toEqual(SELECTION);
    expect(outcome.inputTokens).toBe(42);
    expect(outcome.outputTokens).toBe(7);
    expect(outcome.timeToFirstTokenMs).toBe(150);
    expect(outcome.totalMs).toBe(900);
  });

  it("ignores chunks for other request ids", async () => {
    const request = makeRequest();
    const { api } = scriptedApi([
      { type: "delta", requestId: "req_other", text: "NOISE" },
      { type: "delta", requestId: request.requestId, text: "mine" },
      { type: "completed", requestId: request.requestId, finishReason: "stop", totalMs: 10 },
    ]);
    const outcome = await streamRequest(request, {}, api).done;
    expect(outcome.text).toBe("mine");
  });

  it("resolves with an error outcome when the stream fails", async () => {
    const request = makeRequest();
    const { api } = scriptedApi([
      {
        type: "failed",
        requestId: request.requestId,
        error: { kind: "ai", code: "ai.provider_error", message: "boom", recoverable: true },
      },
    ]);
    const outcome = await streamRequest(request, {}, api).done;
    expect(outcome.finishReason).toBe("error");
    expect(outcome.error?.code).toBe("ai.provider_error");
  });

  it("resolves with an error outcome when the invoke itself rejects", async () => {
    const request = makeRequest();
    const api = {
      ai: {
        stream: async () => {
          throw new Error("transport down");
        },
        cancel: async () => true,
      },
    };
    const outcome = await streamRequest(request, {}, api).done;
    expect(outcome.finishReason).toBe("error");
    expect(outcome.error?.message).toBe("transport down");
  });

  it("cancel() calls the backend and settles as cancelled", async () => {
    const request = makeRequest();
    const { api, cancelled } = scriptedApi([
      { type: "delta", requestId: request.requestId, text: "partial" },
      // no completed chunk — stream hangs until cancel
    ]);
    const handle = streamRequest(request, {}, api);
    await handle.cancel();
    const outcome = await handle.done;
    expect(cancelled).toEqual([request.requestId]);
    expect(outcome.finishReason).toBe("cancelled");
    expect(outcome.text).toBe("partial");
  });

  it("first settle wins: a late completed chunk does not override cancellation", async () => {
    const request = makeRequest();
    const late: { fn?: () => void } = {};
    const api = {
      ai: {
        stream: async (_request: AIRequest, onChunk: (chunk: AIChunk) => void) => {
          late.fn = () =>
            onChunk({ type: "completed", requestId: request.requestId, finishReason: "stop", totalMs: 5 });
        },
        cancel: async () => true,
      },
    };
    const handle = streamRequest(request, {}, api);
    await handle.cancel();
    late.fn?.();
    const outcome = await handle.done;
    expect(outcome.finishReason).toBe("cancelled");
  });
});
