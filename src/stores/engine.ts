/**
 * Holds the singleton ResponseEngine. The real implementation lives in
 * `src/ai/engine.ts` (intelligence layer); until it lands, `createResponseEngine`
 * throws and we substitute a graceful stub so the UI stays usable.
 * Tests inject fakes with `setEngine`.
 */

import { createResponseEngine } from "@/ai/engine";
import type { EngineHandle, ResponseEngine } from "@/lib/engine-contract";
import type { BlueyError } from "@/lib/types";
import { createId } from "@/lib/utils/id";

let instance: ResponseEngine | null = null;

export function setEngine(engine: ResponseEngine | null): void {
  instance = engine;
}

export function getEngine(): ResponseEngine {
  if (!instance) {
    try {
      instance = createResponseEngine();
    } catch (error) {
      console.warn("[engine] createResponseEngine failed; using unavailable stub", error);
      instance = createUnavailableEngine();
    }
  }
  return instance;
}

const UNAVAILABLE_ERROR: BlueyError = {
  kind: "configuration",
  code: "ai.engine_unavailable",
  message: "The response engine is not available in this build.",
  recoverable: true,
  recovery: { type: "open_settings", tab: "ai" },
};

function createUnavailableEngine(): ResponseEngine {
  return {
    ask: (_input, callbacks): EngineHandle => {
      const requestId = createId("req");
      queueMicrotask(() => callbacks?.onError?.(UNAVAILABLE_ERROR, requestId));
      return {
        requestId,
        generation: 0,
        cancel: async () => {},
        done: Promise.resolve(null),
      };
    },
    prepare: async () => null,
    takePrepared: () => null,
    classify: async () => null,
    summarizeSession: async () => {
      throw UNAVAILABLE_ERROR;
    },
    cancelAll: async () => {},
  };
}
