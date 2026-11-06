/**
 * Model-assisted classification: the fast model refines heuristic results
 * only in the 0.4–0.7 confidence band, and only when a fast model is
 * configured. Cheap by construction (tiny schema, ~100 output tokens).
 */

import { CLASSIFY_REFINE_DEADLINE_MS, createResponseEngine, type EngineBus } from "@/ai/engine";
import { setTransport } from "@/lib/tauri/transport";
import type { AIChunk, DetectedEvent } from "@/lib/types";
import { FakeTransport } from "../fixtures/helpers/fake-transport";
import { makeMode, makeSegment, makeSettings } from "../fixtures/helpers/builders";

const interviewMode = makeMode({ id: "interview", responseSchema: "suggested-response", group: "Looking for work" });

// Interrogative lead without a question mark → heuristic confidence 0.58.
const MID_CONFIDENCE_SEGMENT = makeSegment({
  source: "system",
  text: "what happens after the beta program ends for early customers",
});

const settingsWithFastModel = makeSettings({
  ai: { models: {
    default: { providerId: "mock", model: "mock-default" },
    fast: { providerId: "mock", model: "mock-fast" },
    reasoning: null,
    vision: null,
    research: null,
    transcription: null,
    embedding: null,
  } },
});

function refinementScript(json: string) {
  return (request: { requestId: string }, emit: (chunk: AIChunk) => void): void => {
    emit({ type: "delta", requestId: request.requestId, text: json });
    emit({ type: "completed", requestId: request.requestId, finishReason: "stop", totalMs: 60 });
  };
}

describe("classify with fast-model refinement", () => {
  let fake: FakeTransport;

  beforeEach(() => {
    fake = new FakeTransport();
    fake.handle("ai_cancel", () => true);
    setTransport(fake);
  });

  it("refines a mid-confidence heuristic with a tiny classification request", async () => {
    fake.setAIScript(refinementScript('{"type":"question","requiresResponse":true,"confidence":0.85}'));
    const engine = createResponseEngine();
    const event = await engine.classify({
      segment: MID_CONFIDENCE_SEGMENT,
      recent: [],
      mode: interviewMode,
      settings: settingsWithFastModel,
    });

    expect(event).not.toBeNull();
    expect(event!.type).toBe("question");
    expect(event!.confidence).toBe(0.85);
    expect(event!.requiresResponse).toBe(true);

    const request = fake.callsFor("ai_stream")[0]!.request;
    expect(request.task).toBe("classification");
    expect(request.latencyBudget).toBe("ultra-fast");
    expect(request.maxOutputTokens).toBeLessThanOrEqual(150);
    expect(request.outputSchema?.name).toBe("bluey_classification");
    const userPart = request.messages[1]?.content[0];
    const userText = userPart && "text" in userPart ? userPart.text : "";
    expect(userText).toContain(MID_CONFIDENCE_SEGMENT.text);
  });

  it("treats null refinement fields from strict-mode providers as absent", async () => {
    fake.setAIScript(refinementScript('{"type":"technical_question","requiresResponse":null,"confidence":null}'));
    const engine = createResponseEngine();
    const event = await engine.classify({
      segment: MID_CONFIDENCE_SEGMENT,
      recent: [],
      mode: interviewMode,
      settings: settingsWithFastModel,
    });
    expect(event).not.toBeNull();
    expect(event!.type).toBe("technical_question");
    expect(event!.confidence).toBeCloseTo(0.58, 5); // heuristic value kept
  });

  it("drops the event when the model confidently says none", async () => {
    fake.setAIScript(refinementScript('{"type":"none","confidence":0.9}'));
    const engine = createResponseEngine();
    const event = await engine.classify({
      segment: MID_CONFIDENCE_SEGMENT,
      recent: [],
      mode: interviewMode,
      settings: settingsWithFastModel,
    });
    expect(event).toBeNull();
  });

  it("keeps the heuristic when the refinement stream fails", async () => {
    fake.setAIScript((request, emit) => {
      emit({
        type: "failed",
        requestId: request.requestId,
        error: { kind: "ai", code: "ai.timeout", message: "slow", recoverable: true },
      });
    });
    const engine = createResponseEngine();
    const event = await engine.classify({
      segment: MID_CONFIDENCE_SEGMENT,
      recent: [],
      mode: interviewMode,
      settings: settingsWithFastModel,
    });
    expect(event).not.toBeNull();
    expect(event!.confidence).toBeCloseTo(0.58, 5);
  });

  it("never calls the model without a configured fast model", async () => {
    const engine = createResponseEngine();
    const event = await engine.classify({
      segment: MID_CONFIDENCE_SEGMENT,
      recent: [],
      mode: interviewMode,
      settings: makeSettings(), // fast: null
    });
    expect(event).not.toBeNull();
    expect(fake.callsFor("ai_stream")).toHaveLength(0);
  });

  it("skips the model for high-confidence heuristics even with a fast model", async () => {
    const engine = createResponseEngine();
    const event = await engine.classify({
      segment: makeSegment({ source: "system", text: "Why do you want to work here?" }),
      recent: [],
      mode: interviewMode,
      settings: settingsWithFastModel,
    });
    expect(event).not.toBeNull();
    expect(event!.confidence).toBeGreaterThan(0.7);
    expect(fake.callsFor("ai_stream")).toHaveLength(0);
  });

  describe("never delays or reorders a heard question", () => {
    function recordingBus(): { bus: EngineBus; detected: DetectedEvent[] } {
      const detected: DetectedEvent[] = [];
      const bus = {
        emit: (name: string, payload: unknown) => {
          if (name === "question.detected") detected.push(payload as DetectedEvent);
        },
        on: () => () => {},
      } as unknown as EngineBus;
      return { bus, detected };
    }

    /** A refinement that answers only after `delayMs`. */
    function slowRefinement(json: string, delayMs: number) {
      return (request: { requestId: string }, emit: (chunk: AIChunk) => void): void => {
        setTimeout(() => refinementScript(json)(request, emit), delayMs);
      };
    }

    afterEach(() => {
      vi.useRealTimers();
    });

    it("announces the heuristic detection before the model answers", async () => {
      vi.useFakeTimers();
      fake.setAIScript(slowRefinement('{"type":"question","requiresResponse":true,"confidence":0.9}', 300));
      const { bus, detected } = recordingBus();
      const engine = createResponseEngine({ bus });
      const pending = engine.classify({ segment: MID_CONFIDENCE_SEGMENT, recent: [], mode: interviewMode, settings: settingsWithFastModel });
      await vi.advanceTimersByTimeAsync(0);
      expect(detected).toHaveLength(1); // surfaced at once, not after the model
      expect(detected[0]!.confidence).toBeCloseTo(0.58, 5);
      await vi.advanceTimersByTimeAsync(300);
      expect((await pending)!.confidence).toBe(0.9);
      expect(detected).toHaveLength(1); // the refinement never re-announces it
    });

    it("withdraws the suggestion when the model says none within the deadline", async () => {
      vi.useFakeTimers();
      fake.setAIScript(slowRefinement('{"type":"none","confidence":0.9}', 200));
      const { bus, detected } = recordingBus();
      const refuted: string[] = [];
      const engine = createResponseEngine({ bus });
      const pending = engine.classify({
        segment: MID_CONFIDENCE_SEGMENT,
        recent: [],
        mode: interviewMode,
        settings: settingsWithFastModel,
        onRefuted: (event) => refuted.push(event.id),
      });
      await vi.advanceTimersByTimeAsync(200);
      expect(await pending).toBeNull();
      expect(refuted).toEqual([detected[0]!.id]);
    });

    it("ignores a verdict that arrives after the deadline", async () => {
      vi.useFakeTimers();
      fake.setAIScript(slowRefinement('{"type":"none","confidence":0.9}', CLASSIFY_REFINE_DEADLINE_MS + 500));
      const { bus } = recordingBus();
      const refuted: string[] = [];
      const engine = createResponseEngine({ bus });
      const pending = engine.classify({
        segment: MID_CONFIDENCE_SEGMENT,
        recent: [],
        mode: interviewMode,
        settings: settingsWithFastModel,
        onRefuted: (event) => refuted.push(event.id),
      });
      await vi.advanceTimersByTimeAsync(CLASSIFY_REFINE_DEADLINE_MS);
      expect((await pending)!.confidence).toBeCloseTo(0.58, 5); // the heuristic stands
      await vi.advanceTimersByTimeAsync(500);
      expect(refuted).toEqual([]);
    });
  });
});
