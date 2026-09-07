/**
 * Transcript → question detection → prepare() → takePrepared() flow, with
 * proactive-preparation gating and the 3-minute TTL.
 */

import { createResponseEngine, PREPARED_TTL_MS } from "@/ai/engine";
import { setTransport } from "@/lib/tauri/transport";
import { eventBus } from "@/lib/tauri/event-bus";
import type { AIChunk, ContextSnapshot, DetectedEvent } from "@/lib/types";
import { FakeTransport } from "../fixtures/helpers/fake-transport";
import { makeSession, makeSettings } from "../fixtures/helpers/builders";
import { loadFixture } from "../fixtures/helpers/fixtures";

const fixture = loadFixture("interview");

const suggestionJson = JSON.stringify({
  responseType: "suggestion",
  title: "Why Acme",
  content: "I have followed Acme's platform work for a while, and this role sits exactly where I do my best work.",
  sections: [{ title: "Key point", content: "Tie the motivation to the platform team's actual roadmap." }],
});

function suggestionScript(request: { requestId: string }, emit: (chunk: AIChunk) => void): void {
  emit({
    type: "started",
    requestId: request.requestId,
    selection: { providerId: "mock", providerKind: "mock", model: "mock-fast", role: "fast", reason: "test" },
  });
  emit({ type: "delta", requestId: request.requestId, text: suggestionJson });
  emit({ type: "completed", requestId: request.requestId, finishReason: "stop", totalMs: 640, timeToFirstTokenMs: 90 });
}

describe("prepare flow", () => {
  let fake: FakeTransport;
  let clock: { at: Date };

  beforeEach(() => {
    fake = new FakeTransport();
    clock = { at: new Date("2026-09-07T09:00:20.000Z") };
    fake.handle("context_build_snapshot", () => fixture.snapshot as ContextSnapshot);
    fake.handle("documents_retrieve", () => [
      {
        chunkId: "c_res",
        documentId: "d_res",
        documentTitle: "Resume",
        documentKind: "resume" as const,
        content: "Six years building payment platforms; led a team of five.",
        score: 0.86,
        scope: "global" as const,
      },
    ]);
    fake.handle("responses_save", ({ response }) => response);
    fake.handle("sessions_add_event", () => {
      throw new Error("prepare must stay silent — no session events expected");
    });
    fake.handle("ai_cancel", () => true);
    fake.setAIScript(suggestionScript);
    setTransport(fake);
  });

  function makeEngine() {
    return createResponseEngine({ now: () => clock.at });
  }

  async function detectQuestion(engine: ReturnType<typeof makeEngine>): Promise<DetectedEvent> {
    const segments = fixture.transcript;
    const event = await engine.classify({
      segment: segments[segments.length - 1]!,
      recent: segments.slice(0, -1),
      mode: fixture.mode,
      settings: makeSettings(), // no fast model configured → heuristics only
    });
    expect(event).not.toBeNull();
    return event!;
  }

  it("classifies the interviewer question without any model call and emits question.detected", async () => {
    const engine = makeEngine();
    const detected: DetectedEvent[] = [];
    const off = eventBus.on("question.detected", (event) => detected.push(event));
    const event = await detectQuestion(engine);
    off();

    expect(event.type).toBe("question");
    expect(event.requiresResponse).toBe(true);
    expect(event.speaker).toBe("Interviewer");
    expect(fake.callsFor("ai_stream")).toHaveLength(0); // heuristics only
    expect(detected.map((e) => e.id)).toEqual([event.id]);
  });

  it("prepares silently, caches by event id, and takePrepared pops exactly once", async () => {
    const engine = makeEngine();
    const event = await detectQuestion(engine);
    const preparedEvents: string[] = [];
    const off = eventBus.on("response.prepared", (response) => preparedEvents.push(response.id));

    const prepared = await engine.prepare({
      trigger: "detected_event",
      captureScreen: false,
      mode: fixture.mode,
      session: makeSession({ id: "ses_prep", modeId: fixture.mode.id }),
      settings: makeSettings(),
      detectedEvent: event,
    });
    off();

    expect(prepared).not.toBeNull();
    expect(prepared!.prepared).toBe(true);
    expect(prepared!.type).toBe("suggestion");
    expect(prepared!.prompt).toBe(event.text);
    expect(fake.callsFor("ai_stream")).toHaveLength(1);
    expect(fake.callsFor("responses_save")).toHaveLength(0); // silent path
    expect(preparedEvents).toEqual([prepared!.id]);

    // Retrieval used the interview mode's document requirements.
    const retrieveCalls = fake.callsFor("documents_retrieve");
    expect(retrieveCalls).toHaveLength(1);
    expect(retrieveCalls[0]?.query.kinds).toEqual(expect.arrayContaining(["resume", "job_description"]));

    // A second prepare for the same event reuses the cache.
    const again = await engine.prepare({
      trigger: "detected_event",
      captureScreen: false,
      mode: fixture.mode,
      settings: makeSettings(),
      detectedEvent: event,
    });
    expect(again?.id).toBe(prepared!.id);
    expect(fake.callsFor("ai_stream")).toHaveLength(1); // no new stream

    expect(engine.takePrepared(event.id)?.id).toBe(prepared!.id);
    expect(engine.takePrepared(event.id)).toBeNull();
    expect(engine.takePrepared()).toBeNull();
  });

  it("takePrepared() without an id pops the most recent prepared response", async () => {
    const engine = makeEngine();
    const event = await detectQuestion(engine);
    const prepared = await engine.prepare({
      trigger: "detected_event",
      captureScreen: false,
      mode: fixture.mode,
      settings: makeSettings(),
      detectedEvent: event,
    });
    expect(engine.takePrepared()?.id).toBe(prepared!.id);
  });

  it("respects the proactivePreparation setting", async () => {
    const engine = makeEngine();
    const event = await detectQuestion(engine);
    const result = await engine.prepare({
      trigger: "detected_event",
      captureScreen: false,
      mode: fixture.mode,
      settings: makeSettings({ ai: { proactivePreparation: false } }),
      detectedEvent: event,
    });
    expect(result).toBeNull();
    expect(fake.callsFor("ai_stream")).toHaveLength(0);
  });

  it("evicts the oldest prepared response beyond the cache limit of 5", async () => {
    const engine = makeEngine();
    const eventIds: string[] = [];
    for (let i = 0; i < 6; i += 1) {
      clock.at = new Date(clock.at.getTime() + 1000); // distinct timestamps
      const event: DetectedEvent = {
        id: `evt_cache_${i}`,
        type: "question",
        confidence: 0.9,
        requiresResponse: true,
        text: `Question number ${i}?`,
        segmentIds: [`seg_${i}`],
        speaker: "Interviewer",
        detectedAt: clock.at.toISOString(),
      };
      eventIds.push(event.id);
      const prepared = await engine.prepare({
        trigger: "detected_event",
        captureScreen: false,
        mode: fixture.mode,
        settings: makeSettings(),
        detectedEvent: event,
      });
      expect(prepared).not.toBeNull();
    }
    expect(engine.takePrepared(eventIds[0])).toBeNull(); // evicted
    expect(engine.takePrepared(eventIds[1])).not.toBeNull();
    expect(engine.takePrepared(eventIds[5])).not.toBeNull();
  });

  it("expires prepared responses after the TTL", async () => {
    const engine = makeEngine();
    const event = await detectQuestion(engine);
    await engine.prepare({
      trigger: "detected_event",
      captureScreen: false,
      mode: fixture.mode,
      settings: makeSettings(),
      detectedEvent: event,
    });
    clock.at = new Date(clock.at.getTime() + PREPARED_TTL_MS + 1000);
    expect(engine.takePrepared(event.id)).toBeNull();
  });
});
