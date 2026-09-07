/** Shared test setup: fresh MockTransport + stores, and a fake engine. */

import type { AskInput, EngineCallbacks, EngineHandle, ResponseEngine } from "@/lib/engine-contract";
import { eventBus } from "@/lib/tauri/event-bus";
import { MockTransport } from "@/lib/tauri/mock";
import { setTransport } from "@/lib/tauri/transport";
import type { BlueyError, BlueyResponse } from "@/lib/types";
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

  async prepare(): Promise<BlueyResponse | null> {
    return null;
  }

  takePrepared(): BlueyResponse | null {
    return this.preparedQueue.shift() ?? null;
  }

  async classify(): Promise<null> {
    return null;
  }

  summarizeSession(): never {
    throw new Error("not used in tests");
  }

  async cancelAll(): Promise<void> {
    this.cancelled = true;
  }
}
