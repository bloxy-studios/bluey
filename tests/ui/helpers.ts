/** Shared test setup: fresh MockTransport + stores, and a fake engine. */

import type {
  AskInput,
  ClassifyInput,
  EngineCallbacks,
  EngineHandle,
  ResponseEngine,
} from "@/lib/engine-contract";
import { eventBus } from "@/lib/tauri/event-bus";
import { MockTransport } from "@/lib/tauri/mock";
import { setTransport } from "@/lib/tauri/transport";
import type { BlueyError, BlueyResponse, DetectedEvent, TranscriptSegment } from "@/lib/types";
import { setEngine } from "@/stores/engine";
import { initStores, resetStoresForTest } from "@/stores/initStores";

// jsdom lacks navigator.clipboard; provide a spy-able stub.
if (typeof navigator !== "undefined" && !navigator.clipboard) {
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText: async (_text: string) => {} },
    configurable: true,
  });
}

// jsdom lacks element scrolling APIs used by the response thread.
if (typeof Element !== "undefined") {
  if (!("scrollTo" in Element.prototype)) {
    Object.defineProperty(Element.prototype, "scrollTo", { value: () => {}, writable: true });
  }
  if (!("scrollBy" in Element.prototype)) {
    Object.defineProperty(Element.prototype, "scrollBy", { value: () => {}, writable: true });
  }
}

/** Fresh mock transport + re-initialised stores. Call in beforeEach. */
export async function setupMockApp(): Promise<MockTransport> {
  await eventBus.dispose();
  resetStoresForTest();
  setEngine(null);
  const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
  setTransport(mock);
  await initStores();
  return mock;
}

export function makeResponse(partial: Partial<BlueyResponse> = {}): BlueyResponse {
  return {
    id: "resp-1",
    requestId: "req-1",
    modeId: "general",
    type: "answer",
    content: "Hello **world**",
    createdAt: new Date().toISOString(),
    ...partial,
  };
}

/** Controllable ResponseEngine for HUD tests. */
export class FakeEngine implements ResponseEngine {
  asks: AskInput[] = [];
  private callbacks: EngineCallbacks | undefined;
  private resolveDone: ((r: BlueyResponse | null) => void) | null = null;
  cancelled = false;
  preparedQueue: BlueyResponse[] = [];

  ask(input: AskInput, callbacks?: EngineCallbacks): EngineHandle {
    this.asks.push(input);
    this.callbacks = callbacks;
    this.cancelled = false;
    const done = new Promise<BlueyResponse | null>((resolve) => {
      this.resolveDone = resolve;
    });
    callbacks?.onPhase?.("capturing", "req-fake");
    return {
      requestId: "req-fake",
      generation: this.asks.length,
      cancel: async () => {
        this.cancelled = true;
        this.resolveDone?.(null);
      },
      done,
    };
  }

  emitPhase(phase: Parameters<NonNullable<EngineCallbacks["onPhase"]>>[0]): void {
    this.callbacks?.onPhase?.(phase, "req-fake");
  }

  emitDraft(response: BlueyResponse): void {
    this.callbacks?.onDraft?.(response);
  }

  complete(response: BlueyResponse): void {
    this.callbacks?.onComplete?.(response);
    this.resolveDone?.(response);
  }

  fail(error: BlueyError): void {
    this.callbacks?.onError?.(error, "req-fake");
    this.resolveDone?.(null);
  }

  async prepare(_input: AskInput): Promise<BlueyResponse | null> {
    return null;
  }

  /** Event ids ⌘⇧↵ asked for (undefined = "whatever is newest"). */
  takeCalls: Array<string | undefined> = [];

  takePrepared(eventId?: string): BlueyResponse | null {
    this.takeCalls.push(eventId);
    return this.preparedQueue.shift() ?? null;
  }

  async classify(_input: ClassifyInput): Promise<DetectedEvent | null> {
    return null;
  }

  summarizeSession(): never {
    throw new Error("not used in tests");
  }

  async cancelAll(): Promise<void> {
    this.cancelled = true;
  }
}

/**
 * Engine double for the proactive loop: segments ending in "?" classify as a
 * question (emitting `question.detected` like the real engine), and `prepare`
 * emits `response.prepared`. `hold` keeps `prepare` pending until `release()`.
 */
export class ProactiveFakeEngine extends FakeEngine {
  classified: ClassifyInput[] = [];
  prepared: AskInput[] = [];
  hold = false;
  private pending: Array<() => void> = [];

  override async classify(input: ClassifyInput): Promise<DetectedEvent | null> {
    this.classified.push(input);
    if (!input.segment.text.trim().endsWith("?")) return null;
    const event: DetectedEvent = {
      id: `det-${this.classified.length}`,
      type: "question",
      confidence: 0.9,
      requiresResponse: true,
      text: input.segment.text,
      segmentIds: [input.segment.id],
      speaker: input.segment.speaker,
      detectedAt: new Date().toISOString(),
    };
    eventBus.emit("question.detected", event);
    return event;
  }

  override async prepare(input: AskInput): Promise<BlueyResponse | null> {
    this.prepared.push(input);
    if (this.hold) {
      await new Promise<void>((resolve) => {
        this.pending.push(resolve);
      });
    }
    const response = makeResponse({
      id: `prep-${input.detectedEvent?.id ?? "generic"}`,
      prompt: input.detectedEvent?.text,
      content: `Prepared for ${input.detectedEvent?.id ?? "generic"}`,
      prepared: true,
    });
    this.preparedQueue.push(response);
    eventBus.emit("response.prepared", response);
    return response;
  }

  /** Let every held `prepare` call finish. */
  release(): void {
    const pending = this.pending;
    this.pending = [];
    pending.forEach((resolve) => resolve());
  }
}

let segmentCounter = 0;

/** Finalized transcript segment (defaults to the other party over system audio). */
export function makeSegment(partial: Partial<TranscriptSegment> = {}): TranscriptSegment {
  segmentCounter += 1;
  const start = segmentCounter * 4000;
  return {
    id: `seg-${segmentCounter}`,
    source: "system",
    speaker: "Interviewer",
    speakerConfidence: 0.6,
    text: "Tell me about yourself.",
    startTime: start,
    endTime: start + 3000,
    finalized: true,
    createdAt: new Date().toISOString(),
    ...partial,
  };
}
