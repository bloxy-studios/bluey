/**
 * The output budget cut the answer short (`finishReason: "length"`): one
 * retry with double the room, then salvage — and never raw JSON in the HUD
 * (docs/AI_ARCHITECTURE.md › Output budgets).
 */

import { createResponseEngine } from "@/ai/engine";
import { setTransport } from "@/lib/tauri/transport";
import type { AIChunk, AIRequest, BlueyError, BlueyResponse } from "@/lib/types";
import { FakeTransport } from "../fixtures/helpers/fake-transport";
import { makeMode, makeSettings, makeSnapshot } from "../fixtures/helpers/builders";

interface Step {
  text: string;
  finish: "stop" | "length";
}

const FULL = JSON.stringify({
  responseType: "answer",
  title: "Pick",
  content: "B — the composite index covers the predicate.",
});
const CUT = '{"responseType":"answer","title":"Pick","content":"B — the composite index cov';
const CUT_BEFORE_CONTENT = '{"responseType":"answer","title":"Pi';

function scriptFor(steps: Step[]) {
  let call = 0;
  return (request: AIRequest, emit: (chunk: AIChunk) => void): void => {
    const step = steps[Math.min(call, steps.length - 1)]!;
    call += 1;
    emit({
      type: "started",
      requestId: request.requestId,
      selection: { providerId: "mock", providerKind: "mock", model: "mock-1", role: "default", reason: "test" },
    });
    if (step.text.length > 0) emit({ type: "delta", requestId: request.requestId, text: step.text });
    emit({ type: "completed", requestId: request.requestId, finishReason: step.finish, totalMs: 10 });
  };
}

describe("truncated output", () => {
  let fake: FakeTransport;

  beforeEach(() => {
    fake = new FakeTransport();
    fake.handle("documents_retrieve", () => []);
    fake.handle("responses_save", ({ response }) => response);
    fake.handle("ai_cancel", () => true);
    setTransport(fake);
  });

  async function ask(steps: Step[]) {
    fake.setAIScript(scriptFor(steps));
    const engine = createResponseEngine({ now: () => new Date("2026-09-12T10:00:00.000Z") });
    const errors: BlueyError[] = [];
    const handle = engine.ask(
      {
        trigger: "typed",
        instruction: "Which index should I add?",
        captureScreen: false,
        snapshot: makeSnapshot(),
        mode: makeMode(),
        settings: makeSettings(),
      },
      { onError: (error) => errors.push(error) },
    );
    const result: BlueyResponse | null = await handle.done;
    return { result, errors, requests: fake.callsFor("ai_stream").map((call) => call.request) };
  }

  it("retries once with double the budget and uses the complete retry", async () => {
    const { result, errors, requests } = await ask([
      { text: CUT, finish: "length" },
      { text: FULL, finish: "stop" },
    ]);
    expect(errors).toEqual([]);
    expect(requests).toHaveLength(2);
    expect(requests[1]!.requestId).toBe(`${requests[0]!.requestId}_r2`);
    expect(requests[1]!.maxOutputTokens).toBe((requests[0]!.maxOutputTokens ?? 0) * 2);
    expect(requests[1]!.trace).toBeUndefined();
    expect(result?.content).toBe("B — the composite index covers the predicate.");
    expect(result?.truncated).toBeUndefined();
  });

  it("salvages the streamed content and marks the answer truncated when the retry is cut too", async () => {
    const { result, errors, requests } = await ask([{ text: CUT, finish: "length" }]);
    expect(errors).toEqual([]);
    expect(requests).toHaveLength(2);
    expect(result?.content).toBe("B — the composite index cov");
    expect(result?.title).toBe("Pick");
    expect(result?.truncated).toBe(true);
    expect(result?.content).not.toContain("{");
  });

  it("fails with ai.truncated when nothing readable was streamed", async () => {
    const { result, errors } = await ask([{ text: CUT_BEFORE_CONTENT, finish: "length" }]);
    expect(result).toBeNull();
    expect(errors.map((error) => error.code)).toEqual(["ai.truncated"]);
    expect(errors[0]?.recovery).toEqual({ type: "retry" });
  });

  it("fails with ai.unreadable_output instead of showing an empty envelope", async () => {
    const { result, errors, requests } = await ask([
      { text: '{"responseType":"answer","title":"x","content":""}', finish: "stop" },
    ]);
    expect(requests).toHaveLength(1);
    expect(result).toBeNull();
    expect(errors.map((error) => error.code)).toEqual(["ai.unreadable_output"]);
  });
});
