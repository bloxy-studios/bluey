/**
 * End-to-end protocol tests: drive the whole stdio loop in-process through
 * `startSidecar` with injected streams. Mock mode / injected queryFn keep this
 * fully offline — no network, no model, no CLI subprocess.
 */

import { PassThrough } from "node:stream";
import { describe, expect, it } from "vitest";

import { sanitizeErrorMessage, type QueryFn } from "../../sidecars/agent/src/agent";
import type { BuildVariant } from "../../sidecars/agent/src/config";
import type { GenerateFn } from "../../sidecars/agent/src/gemini";
import { startSidecar, type StartSidecarOptions } from "../../sidecars/agent/src/main";

type Frame = Record<string, unknown>;

interface Harness {
  frames: Frame[];
  done: Promise<number>;
  send(frame: Record<string, unknown>): void;
  sendRaw(line: string): void;
  waitFor(pred: (frame: Frame) => boolean, label: string): Promise<Frame>;
  eventsNamed(name: string): Frame[];
}

function makeHarness(options: Omit<StartSidecarOptions, "input" | "output"> = {}): Harness {
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
    send: (frame) => input.write(`${JSON.stringify(frame)}\n`),
    sendRaw: (line) => input.write(`${line}\n`),
    waitFor: (pred, label) => {
      const existing = frames.find(pred);
      if (existing) return Promise.resolve(existing);
      return new Promise<Frame>((resolve, reject) => {
        const timer = setTimeout(() => {
          reject(new Error(`timed out waiting for ${label}; saw: ${JSON.stringify(frames, null, 2)}`));
        }, 5_000);
        waiters.push({
          pred,
          resolve: (frame) => {
            clearTimeout(timer);
            resolve(frame);
          },
        });
      });
    },
    eventsNamed: (name) => frames.filter((f) => f["event"] === name),
  };
}

const isEvent =
  (name: string) =>
  (frame: Frame): boolean =>
    frame["event"] === name;

const baseRun = {
  id: 1,
  method: "research.run",
  params: {
    jobId: "job-1",
    query: "public query about bun",
    goal: "explain bun sidecars",
    tools: ["exa_search", "firecrawl_scrape"],
  },
};

function abortError(): Error {
  const err = new Error("aborted");
  err.name = "AbortError";
  return err;
}

/** init + hang until the job's AbortController fires. */
const hangingQueryFn: QueryFn = ({ options }) => ({
  async *[Symbol.asyncIterator]() {
    yield { type: "system", subtype: "init", model: "hanging-model", tools: [] };
    await new Promise<never>((_resolve, reject) => {
      const signal = options.abortController?.signal;
      if (!signal) return;
      if (signal.aborted) {
        reject(abortError());
        return;
      }
      signal.addEventListener("abort", () => reject(abortError()), { once: true });
    });
  },
});

const mockCases: Array<{ backend: string; env: Record<string, string>; buildVariant?: BuildVariant }> = [
  { backend: "gemini", env: { BLUEY_AGENT_MOCK: "1" } },
  { backend: "claude", env: { BLUEY_AGENT_MOCK: "1", RESEARCH_BACKEND: "claude" } },
  // The lite build's Claude pre-flight must not get in the way of mock mode (no CLI is spawned).
  {
    backend: "claude, lite build",
    env: { BLUEY_AGENT_MOCK: "1", RESEARCH_BACKEND: "claude" },
    buildVariant: "lite",
  },
];

describe.each(mockCases)("sidecar end-to-end (mock mode, $backend backend)", ({ env, buildVariant }) => {
  it("runs a full research job over the protocol and exits cleanly", async () => {
    const harness = makeHarness({ env, buildVariant });
    harness.send(baseRun);

    const accepted = await harness.waitFor((f) => f["id"] === 1, "accepted response");
    expect(accepted["result"]).toEqual({ accepted: true });

    const completedFrame = await harness.waitFor(isEvent("research.completed"), "completed");
    expect(await harness.done).toBe(0);

    // Event choreography
    const names = harness.frames.filter((f) => f["event"]).map((f) => f["event"]);
    expect(names[0]).toBe("research.started");
    expect(names).toContain("research.progress");
    expect(names).toContain("research.toolCall");
    expect(names).toContain("research.textDelta");
    expect(names[names.length - 1]).toBe("research.completed");

    const started = harness.eventsNamed("research.started")[0]!["data"] as Frame;
    expect(started["jobId"]).toBe("job-1");
    expect(typeof started["model"]).toBe("string");

    const toolCalls = harness.eventsNamed("research.toolCall").map((f) => (f["data"] as Frame)["tool"]);
    expect(toolCalls).toEqual(["exa_search", "firecrawl_scrape"]);

    // Deltas stream the same text as the final report
    const deltas = harness
      .eventsNamed("research.textDelta")
      .map((f) => ((f["data"] as Frame)["text"] as string) ?? "")
      .join("");
    const completed = completedFrame["data"] as Frame;
    expect(completed["report"]).toBe(deltas);

    // Citations: deduped, only tool-observed URLs, model citation first
    const citations = completed["citations"] as Array<{ title: string; url: string }>;
    expect(citations.length).toBeGreaterThanOrEqual(2);
    expect(citations[0]?.url).toBe("https://example.org/bluey/overview");
    const urls = citations.map((c) => c.url);
    expect(new Set(urls).size).toBe(urls.length);

    expect(typeof completed["turns"]).toBe("number");
    expect(typeof completed["totalMs"]).toBe("number");
    expect(completed["usage"]).toEqual({ inputTokens: 1200, outputTokens: 300 });
  });

  it("serves document_read through the document.request/response round-trip", async () => {
    const harness = makeHarness({ env, buildVariant });
    harness.send({
      ...baseRun,
      params: {
        ...baseRun.params,
        tools: ["exa_search", "document_read"],
        allowedDocumentIds: ["doc-1"],
      },
    });

    const request = await harness.waitFor(isEvent("document.request"), "document.request");
    const data = request["data"] as Frame;
    expect(data["documentId"]).toBe("doc-1");

    harness.send({
      id: 99,
      method: "document.response",
      params: {
        requestId: data["requestId"],
        documentId: "doc-1",
        text: "local document body",
      },
    });

    await harness.waitFor(isEvent("research.completed"), "completed");
    expect(await harness.done).toBe(0);

    const progress = harness
      .eventsNamed("research.progress")
      .map((f) => (f["data"] as Frame)["message"] as string);
    expect(progress.some((m) => m.includes('document_read: loaded "doc-1"'))).toBe(true);
    // document.response is fire-and-forget: no `{id: 99}` reply frame
    expect(harness.frames.find((f) => f["id"] === 99)).toBeUndefined();
  });
});

/**
 * Enough env for a Claude-backend job to start when the query itself is stubbed
 * out (`queryFn` only drives the Claude backend, so the backend is explicit).
 */
const stubKeys = {
  RESEARCH_BACKEND: "claude",
  ANTHROPIC_API_KEY: "test-anthropic",
  EXA_API_KEY: "test-exa",
  FIRECRAWL_API_KEY: "test-firecrawl",
};

describe("sidecar cancellation", () => {
  it("maps research.cancel onto abort → research.failed { code: cancelled, kind: cancelled }", async () => {
    const harness = makeHarness({ env: { ...stubKeys }, deps: { queryFn: hangingQueryFn } });
    harness.send(baseRun);

    await harness.waitFor(
      (f) => isEvent("research.progress")(f) || isEvent("research.started")(f),
      "job start",
    );
    harness.send({ id: 2, method: "research.cancel", params: { jobId: "job-1" } });

    const cancelReply = await harness.waitFor((f) => f["id"] === 2, "cancel reply");
    expect(cancelReply["result"]).toEqual({ cancelled: true });

    const failed = await harness.waitFor(isEvent("research.failed"), "failed event");
    const error = (failed["data"] as Frame)["error"] as Frame;
    expect(error["code"]).toBe("cancelled");
    expect(error["kind"]).toBe("cancelled");
    expect(await harness.done).toBe(0);
  });

  it("rejects cancels for unknown jobs", async () => {
    const harness = makeHarness({ env: { BLUEY_AGENT_MOCK: "1" } });
    harness.send({ id: 5, method: "research.cancel", params: { jobId: "nope" } });
    const reply = await harness.waitFor((f) => f["id"] === 5, "cancel error");
    expect((reply["error"] as Frame)["code"]).toBe("unknown_job");
  });
});

describe("sidecar configuration failures", () => {
  it("by default (Gemini) fails with missing_api_key naming GEMINI_API_KEY (never a value)", async () => {
    const harness = makeHarness({ env: { ANTHROPIC_API_KEY: "sk-not-a-gemini-key" } });
    harness.send(baseRun);

    const failed = await harness.waitFor(isEvent("research.failed"), "failed event");
    const error = (failed["data"] as Frame)["error"] as Frame;
    expect(error["code"]).toBe("missing_api_key");
    expect(error["kind"]).toBe("configuration");
    expect(error["message"]).toContain("GEMINI_API_KEY");
    expect(JSON.stringify(harness.frames)).not.toContain("sk-not-a-gemini-key");
    expect(await harness.done).toBe(0);
  });

  it("with RESEARCH_BACKEND=claude fails with missing_api_key naming ANTHROPIC_API_KEY", async () => {
    const harness = makeHarness({ env: { RESEARCH_BACKEND: "claude", GEMINI_API_KEY: "AIza-secret" } });
    harness.send(baseRun);

    const failed = await harness.waitFor(isEvent("research.failed"), "failed event");
    const error = (failed["data"] as Frame)["error"] as Frame;
    expect(error["code"]).toBe("missing_api_key");
    expect(error["message"]).toContain("ANTHROPIC_API_KEY");
    expect(JSON.stringify(harness.frames)).not.toContain("AIza-secret");
    expect(await harness.done).toBe(0);
  });

  it("fails with missing_api_key naming EXA_API_KEY when the tool needs it", async () => {
    const harness = makeHarness({ env: { GEMINI_API_KEY: "AIza-test-secret" } });
    harness.send({ ...baseRun, params: { ...baseRun.params, tools: ["exa_search"] } });

    const failed = await harness.waitFor(isEvent("research.failed"), "failed event");
    const error = (failed["data"] as Frame)["error"] as Frame;
    expect(error["code"]).toBe("missing_api_key");
    expect(error["message"]).toContain("EXA_API_KEY");
    expect(JSON.stringify(harness.frames)).not.toContain("AIza-test-secret");
    expect(await harness.done).toBe(0);
  });

  it("on Microsoft Foundry, fails with missing_api_key naming ANTHROPIC_FOUNDRY_API_KEY", async () => {
    // stubKeys carries ANTHROPIC_API_KEY — an Anthropic-direct key is not a Foundry credential.
    const harness = makeHarness({
      env: { ...stubKeys, CLAUDE_CODE_USE_FOUNDRY: "1", ANTHROPIC_FOUNDRY_RESOURCE: "my-res" },
    });
    harness.send(baseRun);

    const failed = await harness.waitFor(isEvent("research.failed"), "failed event");
    const error = (failed["data"] as Frame)["error"] as Frame;
    expect(error["code"]).toBe("missing_api_key");
    expect(error["kind"]).toBe("configuration");
    expect(error["message"]).toContain("ANTHROPIC_FOUNDRY_API_KEY");
    expect(error["message"]).not.toContain("ANTHROPIC_API_KEY is not set");
    expect(JSON.stringify(harness.frames)).not.toContain("test-anthropic");
    expect(await harness.done).toBe(0);
  });

  it("on the lite build, RESEARCH_BACKEND=claude without BLUEY_CLAUDE_CLI fails with invalid_configuration naming BLUEY_CLAUDE_CLI", async () => {
    // Credentials are fine; only the CLI is missing. Without the pre-flight this would reach
    // the SDK, which throws a raw "Native CLI binary … not found" error.
    const harness = makeHarness({
      env: { ...stubKeys, ANTHROPIC_API_KEY: "sk-lite-secret" },
      buildVariant: "lite",
    });
    harness.send(baseRun);

    const failed = await harness.waitFor(isEvent("research.failed"), "failed event");
    const error = (failed["data"] as Frame)["error"] as Frame;
    expect(error["code"]).toBe("invalid_configuration");
    expect(error["kind"]).toBe("configuration");
    expect(error["message"]).toContain("BLUEY_CLAUDE_CLI");
    expect(error["message"]).toContain("BLUEY_AGENT_VARIANT=full");
    expect(JSON.stringify(harness.frames)).not.toContain("sk-lite-secret");
    // Nothing but started + failed: no agent session was attempted.
    expect(harness.frames.filter((f) => f["event"]).map((f) => f["event"])).toEqual([
      "research.started",
      "research.failed",
    ]);
    expect(await harness.done).toBe(0);
  });

  it("on Microsoft Foundry, fails with invalid_configuration when no resource/base URL is set", async () => {
    const harness = makeHarness({
      env: { ...stubKeys, CLAUDE_CODE_USE_FOUNDRY: "1", ANTHROPIC_FOUNDRY_API_KEY: "foundry-secret" },
    });
    harness.send(baseRun);

    const failed = await harness.waitFor(isEvent("research.failed"), "failed event");
    const error = (failed["data"] as Frame)["error"] as Frame;
    expect(error["code"]).toBe("invalid_configuration");
    expect(error["kind"]).toBe("configuration");
    expect(error["message"]).toContain("ANTHROPIC_FOUNDRY_RESOURCE");
    expect(JSON.stringify(harness.frames)).not.toContain("foundry-secret");
    expect(await harness.done).toBe(0);
  });
});

describe("sidecar error hygiene", () => {
  const leakedKey = "AIzaSyD-1234567890abcdefghijklmnopqrstuv";

  it("sanitizeErrorMessage strips JSON bodies and key-like values and caps the length", () => {
    const raw =
      'got status: 400 . {"error":{"code":400,"message":"API key not valid. Prompt was: SECRET PROMPT","status":"INVALID_ARGUMENT"}} ' +
      `url https://g.example/v1beta/models/x:streamGenerateContent?alt=sse&key=${leakedKey} ` +
      `Authorization: Bearer sk-ant-api03-${"x".repeat(40)} ` +
      "y".repeat(500);
    const clean = sanitizeErrorMessage(raw);
    expect(clean.length).toBeLessThanOrEqual(200);
    expect(clean).toContain("got status: 400");
    expect(clean).not.toContain("{");
    expect(clean).not.toContain("SECRET PROMPT");
    expect(clean).not.toContain("AIza");
    expect(clean).not.toContain("sk-ant");
    expect(clean).toContain("key=[redacted]");
    expect(clean).toContain("Bearer [redacted]");
    expect(clean.endsWith("…")).toBe(true);
    // Short, harmless messages pass through unchanged; blanks get a placeholder.
    expect(sanitizeErrorMessage("socket hang up")).toBe("socket hang up");
    expect(sanitizeErrorMessage("   ")).toBe("unknown error");
    expect(sanitizeErrorMessage("x-goog-api-key=abc123 token=t0k3n")).toBe(
      "x-goog-api-key=[redacted] token=[redacted]",
    );
  });

  it("forwards status-less Gemini SDK errors as agent_execution_failed with a sanitized message", async () => {
    // No `status` → mapGeminiError leaves it alone → the generic fallback path.
    const leaky: GenerateFn = async () => {
      throw new Error(
        'TypeError: fetch failed {"body":"SECRET PROMPT ECHO"} while calling ' +
          `https://g.example/models?key=${leakedKey} ` +
          "z".repeat(400),
      );
    };
    const harness = makeHarness({
      env: { GEMINI_API_KEY: leakedKey, EXA_API_KEY: "e", FIRECRAWL_API_KEY: "f" },
      deps: { generateFn: leaky },
    });
    harness.send(baseRun);

    const failed = await harness.waitFor(isEvent("research.failed"), "failed event");
    const error = (failed["data"] as Frame)["error"] as Frame;
    expect(error["code"]).toBe("agent_execution_failed");
    expect(error["kind"]).toBe("research");
    const message = error["message"] as string;
    expect(message.length).toBeLessThanOrEqual(200);
    expect(message).toContain("fetch failed");
    expect(message).not.toContain("SECRET PROMPT ECHO");
    expect(message).not.toContain("{");
    expect(JSON.stringify(harness.frames)).not.toContain(leakedKey);
    expect(await harness.done).toBe(0);
  });
});

describe("sidecar Microsoft Foundry routing", () => {
  it("hands Claude Code the Foundry variables and strips Anthropic-direct routing", async () => {
    let seenEnv: Record<string, string | undefined> | undefined;
    const capturingQueryFn: QueryFn = ({ options }) => {
      seenEnv = options.env;
      return hangingQueryFn({ prompt: "", options });
    };
    const harness = makeHarness({
      env: {
        ...stubKeys,
        ANTHROPIC_BASE_URL: "https://my-res.services.ai.azure.com/api/projects/proj-default",
        AZURE_FOUNDRY_ENDPOINT: "https://my-res.openai.azure.com",
        AZURE_FOUNDRY_API_KEY: "shared-foundry-secret",
        CLAUDE_CODE_USE_FOUNDRY: "1",
        ANTHROPIC_DEFAULT_OPUS_MODEL: "claude-opus-5",
        ANTHROPIC_DEFAULT_HAIKU_MODEL: "claude-haiku-4-5",
        BLUEY_RESEARCH_MODEL: "claude-opus-5",
      },
      deps: { queryFn: capturingQueryFn },
    });
    harness.send(baseRun);

    const started = await harness.waitFor(isEvent("research.started"), "started");
    expect((started["data"] as Frame)["model"]).toBe("claude-opus-5");
    await harness.waitFor(isEvent("research.progress"), "agent session init");

    expect(seenEnv).toBeDefined();
    const env = seenEnv!;
    expect(env["CLAUDE_CODE_USE_FOUNDRY"]).toBe("1");
    // Derived from AZURE_FOUNDRY_ENDPOINT / AZURE_FOUNDRY_API_KEY (same Foundry resource).
    expect(env["ANTHROPIC_FOUNDRY_RESOURCE"]).toBe("my-res");
    expect(env["ANTHROPIC_FOUNDRY_API_KEY"]).toBe("shared-foundry-secret");
    expect(env["ANTHROPIC_FOUNDRY_BASE_URL"]).toBeUndefined();
    expect(env["ANTHROPIC_DEFAULT_OPUS_MODEL"]).toBe("claude-opus-5");
    expect(env["ANTHROPIC_DEFAULT_HAIKU_MODEL"]).toBe("claude-haiku-4-5");
    // The stray project URL and the Anthropic-direct key never reach Claude Code.
    expect(env["ANTHROPIC_BASE_URL"]).toBeUndefined();
    expect(env["ANTHROPIC_API_KEY"]).toBeUndefined();
    // Nothing secret on the wire.
    expect(JSON.stringify(harness.frames)).not.toContain("shared-foundry-secret");

    harness.send({ id: 2, method: "research.cancel", params: { jobId: "job-1" } });
    await harness.waitFor(isEvent("research.failed"), "cancelled");
    expect(await harness.done).toBe(0);
  });
});

describe("sidecar protocol errors", () => {
  it("answers unknown methods, invalid params, invalid JSON and duplicate runs", async () => {
    const harness = makeHarness({ env: { ...stubKeys }, deps: { queryFn: hangingQueryFn } });

    harness.send({ id: 10, method: "does.not.exist" });
    const unknown = await harness.waitFor((f) => f["id"] === 10, "unknown method");
    expect((unknown["error"] as Frame)["code"]).toBe("unknown_method");

    harness.send({ id: 11, method: "research.run", params: { jobId: "x" } });
    const invalid = await harness.waitFor((f) => f["id"] === 11, "invalid params");
    expect((invalid["error"] as Frame)["code"]).toBe("invalid_params");

    harness.sendRaw("this is not json");
    const badJson = await harness.waitFor(
      (f) => f["id"] === null && f["error"] !== undefined,
      "invalid JSON error",
    );
    expect((badJson["error"] as Frame)["code"]).toBe("invalid_request");

    // First valid run occupies the process; a second one is refused.
    harness.send(baseRun);
    await harness.waitFor((f) => f["id"] === 1, "first run accepted");
    harness.send({ ...baseRun, id: 12 });
    const dup = await harness.waitFor((f) => f["id"] === 12, "duplicate run refused");
    expect((dup["error"] as Frame)["code"]).toBe("job_already_running");

    harness.send({ id: 13, method: "research.cancel", params: { jobId: "job-1" } });
    await harness.waitFor(isEvent("research.failed"), "cancelled");
    expect(await harness.done).toBe(0);
  });
});
