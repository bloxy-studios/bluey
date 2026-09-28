/**
 * The engine's live-suggestion contract: a live prepare can be cancelled like an ask
 * (LIVE-001), every request names its supersede scope, background flag and mode role
 * (LIVE-002, LIVE-003, MODE-012), and prepared answers are saved once shown (DATA-007).
 */

import { createResponseEngine } from "@/ai/engine";
import type { CancelHandle, EnginePhase } from "@/lib/engine-contract";
import { setTransport } from "@/lib/tauri/transport";
import type { AIChunk, AIRequest, ContextSnapshot, DetectedEvent } from "@/lib/types";
import { deferred, FakeTransport } from "../fixtures/helpers/fake-transport";
import { makeMode, makeSession, makeSettings } from "../fixtures/helpers/builders";

const snapshot: ContextSnapshot = {
  timestamp: "2026-09-07T09:00:00.000Z",
  transcript: { segments: [], windowSeconds: 180 },
};

const question: DetectedEvent = {
  id: "evt_q1",
  type: "question",
  text: "Why do you want to work here?",
  speaker: "Interviewer",
  confidence: 0.9,
  requiresResponse: true,
  segmentIds: ["seg_1"],
  detectedAt: "2026-09-07T09:00:01.000Z",
};

function answerScript(request: AIRequest, emit: (chunk: AIChunk) => void): void {
  emit({
    type: "started",
    requestId: request.requestId,
    selection: { providerId: "openai", providerKind: "anthropic", model: "gpt-x", role: "fast", reason: "role fast" },
  });
  emit({ type: "delta", requestId: request.requestId, text: '{"responseType":"answer","content":"Because."}' });
  emit({ type: "completed", requestId: request.requestId, finishReason: "stop", totalMs: 10 });
}

function setup(): FakeTransport {
  const fake = new FakeTransport();
  fake.handle("context_build_snapshot", () => snapshot);
  fake.handle("responses_save", ({ response }) => response);
  fake.handle("sessions_add_event", (args) => ({ id: "ev_1", createdAt: "t", ...args }) as never);
  fake.handle("ai_cancel", () => true);
  fake.setAIScript(answerScript);
  setTransport(fake);
  return fake;
}

const baseInput = () => ({
  trigger: "detected_event" as const,
  captureScreen: false,
  mode: makeMode(),
  settings: makeSettings({ ai: { proactivePreparation: true } }),
  detectedEvent: question,
});

describe("live prepare", () => {
  it("a live suggestion is cancellable through its handle: nothing is shown or saved", async () => {
    const fake = setup();
    const started = deferred();
    fake.setAIScript((request, emit) => {
      emit({ type: "delta", requestId: request.requestId, text: '{"responseType":"answer","content":"Beca' });
      started.resolve();
    });
    const engine = createResponseEngine();
    let handle: CancelHandle | undefined;
    const phases: EnginePhase[] = [];
    const completed: unknown[] = [];
    const pending = engine.prepare(baseInput(), {
      onHandle: (h) => (handle = h),
      onPhase: (phase) => phases.push(phase),
      onComplete: (response) => completed.push(response),
    });

    await started.promise;
    await handle?.cancel();

    expect(await pending).toBeNull();
    expect(phases.at(-1)).toBe("cancelled");
    expect(completed).toHaveLength(0);
    expect(fake.callsFor("ai_cancel").map((c) => c.requestId)).toContain(handle?.requestId);
    expect(fake.callsFor("responses_save")).toHaveLength(0);
  });

  it("tags each request with its supersede scope, background flag and mode role", async () => {
    const fake = setup();
    const engine = createResponseEngine();
    const mode = makeMode({ preferredModelRole: "reasoning" });
    await engine.ask({ trigger: "typed", instruction: "Hi?", captureScreen: false, mode, settings: makeSettings() })
      .done;
    await engine.prepare({ ...baseInput(), mode }, { onComplete: () => {} });
    await engine.prepare({ ...baseInput(), detectedEvent: { ...question, id: "evt_q2" } });

    const sent = fake.callsFor("ai_stream").map(({ request }) => request);
    expect(sent.map((r) => [r.scope, r.background])).toEqual([
      ["ask", false],
      ["live", true],
      ["prepare", true],
    ]);
    expect(sent[0]?.preferredModelRole).toBe("reasoning");
    expect(sent[1]?.preferredModelRole).toBe("reasoning");
  });
  it("a prepared answer opened live is saved and logged like any shown answer", async () => {
    const fake = setup();
    const engine = createResponseEngine();
    const session = makeSession();
    const cached = await engine.prepare({ ...baseInput(), session });
    expect(cached?.prepared).toBe(true);
    expect(fake.callsFor("responses_save")).toHaveLength(0);

    const shown = await engine.prepare({ ...baseInput(), session }, { onComplete: () => {} });

    expect(shown?.id).toBe(cached?.id);
    expect(shown?.prepared).toBeUndefined();
    expect(fake.callsFor("responses_save").map((c) => c.response.id)).toEqual([cached?.id]);
    expect(fake.callsFor("sessions_add_event").map((c) => c.type)).toEqual(["response_generated"]);
  });

  it("commitShown persists a prepared answer shown with the shortcut", async () => {
    const fake = setup();
    const engine = createResponseEngine();
    await engine.prepare({ ...baseInput(), session: makeSession() });
    const taken = engine.takePrepared();
    expect(taken).not.toBeNull();

    const committed = await engine.commitShown(taken!, makeSession());

    expect(committed.prepared).toBeUndefined();
    expect(fake.callsFor("responses_save")[0]?.response.prepared).toBeUndefined();
    expect(fake.callsFor("sessions_add_event")[0]?.refs).toEqual({
      responseId: taken?.id,
      requestId: taken?.requestId,
    });
  });

  it("clearPrepared drops answers written for the previous mode", async () => {
    setup();
    const engine = createResponseEngine();
    await engine.prepare(baseInput());
    engine.clearPrepared();
    expect(engine.takePrepared(question.id)).toBeNull();
  });

  it("carries which model answered, and the router's reason when it fell back", async () => {
    const fake = setup();
    fake.setAIScript((request, emit) => {
      emit({
        type: "started",
        requestId: request.requestId,
        selection: {
          providerId: "openai",
          providerKind: "anthropic",
          model: "gpt-x",
          role: "default",
          reason: "mode role reasoning; role reasoning unassigned → fallback default",
        },
      });
      emit({ type: "delta", requestId: request.requestId, text: '{"responseType":"answer","content":"Hi."}' });
      emit({ type: "completed", requestId: request.requestId, finishReason: "stop", totalMs: 10 });
    });
    const engine = createResponseEngine();
    const response = await engine.ask({ ...baseInput(), trigger: "typed", instruction: "Hi?" }).done;
    expect(response?.selection).toEqual({
      role: "default",
      providerId: "openai",
      model: "gpt-x",
      fallbackReason: "mode role reasoning; role reasoning unassigned → fallback default",
    });
  });
});
