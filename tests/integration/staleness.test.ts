/**
 * Stale-response protection (spec §61–62): when a second ask starts while the
 * first is still streaming, the first is cancelled and its output never
 * reaches the UI or storage.
 */

import { createResponseEngine } from "@/ai/engine";
import type { EnginePhase } from "@/lib/engine-contract";
import { setTransport } from "@/lib/tauri/transport";
import type { BlueyResponse, ContextSnapshot } from "@/lib/types";
import { deferred, FakeTransport } from "../fixtures/helpers/fake-transport";
import { makeMode, makeSettings } from "../fixtures/helpers/builders";

const snapshot: ContextSnapshot = {
  timestamp: "2026-09-07T09:00:00.000Z",
  transcript: {
    segments: [
      {
        id: "seg_1",
        source: "system",
        text: "What port does Postgres use by default?",
        startTime: 0,
        endTime: 2500,
        finalized: true,
        createdAt: "2026-09-07T09:00:02.500Z",
      },
    ],
    windowSeconds: 180,
  },
};

describe("generation gate staleness", () => {
  it("a newer ask supersedes the in-flight one; the stale stream never overwrites", async () => {
    const fake = new FakeTransport();
    const saved: BlueyResponse[] = [];
    fake.handle("context_build_snapshot", () => snapshot);
    fake.handle("responses_save", ({ response }) => {
      saved.push(response);
      return response;
    });
    fake.handle("ai_cancel", () => true);

    const firstStreamStarted = deferred();
    const releaseFirst = deferred();
    let streamCount = 0;
    fake.setAIScript(async (request, emit) => {
      streamCount += 1;
      const mine = streamCount;
      emit({
        type: "started",
        requestId: request.requestId,
        selection: { providerId: "mock", providerKind: "mock", model: "mock-1", role: "default", reason: "t" },
      });
      if (mine === 1) {
        emit({ type: "delta", requestId: request.requestId, text: '{"responseType":"answer","content":"STALE ANSWER"}' });
        firstStreamStarted.resolve();
        await releaseFirst.promise; // finish only after the second ask completed
        emit({ type: "completed", requestId: request.requestId, finishReason: "stop", totalMs: 999 });
      } else {
        emit({ type: "delta", requestId: request.requestId, text: '{"responseType":"answer","content":"Port 5432."}' });
        emit({ type: "completed", requestId: request.requestId, finishReason: "stop", totalMs: 42 });
      }
    });
    setTransport(fake);

    const engine = createResponseEngine();
    const input = {
      trigger: "typed" as const,
      instruction: "What port does Postgres use?",
      captureScreen: false,
      mode: makeMode(),
      settings: makeSettings(),
    };

    const phases1: EnginePhase[] = [];
    let completed1: BlueyResponse | null = null;
    const handle1 = engine.ask(input, {
      onPhase: (phase) => phases1.push(phase),
      onComplete: (response) => {
        completed1 = response;
      },
    });

    await firstStreamStarted.promise;

    let completed2: BlueyResponse | null = null;
    const handle2 = engine.ask(input, {
      onComplete: (response) => {
        completed2 = response;
      },
    });

    expect(handle2.generation).toBeGreaterThan(handle1.generation);

    const result2 = await handle2.done;
    expect(result2).not.toBeNull();
    expect(result2!.content).toBe("Port 5432.");
    expect(completed2).toBe(result2);

    // Now let the stale stream finish — it must be discarded.
    releaseFirst.resolve();
    const result1 = await handle1.done;
    expect(result1).toBeNull();
    expect(completed1).toBeNull();
    expect(phases1[phases1.length - 1]).toBe("cancelled");

    // Starting ask #2 cancelled the in-flight request of ask #1.
    const cancelCalls = fake.callsFor("ai_cancel");
    expect(cancelCalls.map((c) => c.requestId)).toContain(handle1.requestId);

    // Only the fresh response was persisted.
    expect(saved).toHaveLength(1);
    expect(saved[0]?.content).toBe("Port 5432.");
    expect(saved[0]?.requestId).toBe(handle2.requestId);
  });

  it("EngineHandle.cancel() aborts the request and resolves done with null", async () => {
    const fake = new FakeTransport();
    fake.handle("context_build_snapshot", () => snapshot);
    fake.handle("responses_save", ({ response }) => response);
    fake.handle("ai_cancel", () => true);

    const started = deferred();
    fake.setAIScript(async (request, emit) => {
      emit({
        type: "started",
        requestId: request.requestId,
        selection: { providerId: "mock", providerKind: "mock", model: "mock-1", role: "default", reason: "t" },
      });
      emit({ type: "delta", requestId: request.requestId, text: "partial" });
      started.resolve();
      // never completes — engine-side cancel must settle it
    });
    setTransport(fake);

    const engine = createResponseEngine();
    const phases: EnginePhase[] = [];
    const handle = engine.ask(
      {
        trigger: "typed",
        instruction: "hang forever",
        captureScreen: false,
        mode: makeMode(),
        settings: makeSettings(),
      },
      { onPhase: (phase) => phases.push(phase) },
    );

    await started.promise;
    await handle.cancel();
    const result = await handle.done;

    expect(result).toBeNull();
    expect(phases[phases.length - 1]).toBe("cancelled");
    expect(fake.callsFor("ai_cancel").map((c) => c.requestId)).toContain(handle.requestId);
    expect(fake.callsFor("responses_save")).toHaveLength(0);
  });

  it("cancelAll() invalidates in-flight work via ai_cancel_all", async () => {
    const fake = new FakeTransport();
    fake.handle("context_build_snapshot", () => snapshot);
    fake.handle("responses_save", ({ response }) => response);
    fake.handle("ai_cancel", () => true);
    fake.handle("ai_cancel_all", () => 1);

    const started = deferred();
    fake.setAIScript(async (request, emit) => {
      emit({ type: "delta", requestId: request.requestId, text: "partial" });
      started.resolve();
    });
    setTransport(fake);

    const engine = createResponseEngine();
    const handle = engine.ask({
      trigger: "typed",
      instruction: "will be globally cancelled",
      captureScreen: false,
      mode: makeMode(),
      settings: makeSettings(),
    });

    await started.promise;
    await engine.cancelAll();
    expect(fake.callsFor("ai_cancel_all")).toHaveLength(1);

    // The pipeline notices the bumped generation once its stream would settle.
    await handle.cancel(); // settle the hanging stream
    expect(await handle.done).toBeNull();
  });
});
