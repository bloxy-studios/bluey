/** Assemble `ResponseMetrics` and publish `dev.metrics` locally. */

import { eventBus } from "@/lib/tauri/event-bus";
import type {
  ContextSnapshot,
  LatencyMetrics,
  ModelSelection,
  ResponseMetrics,
} from "@/lib/types";
import type { StreamOutcome } from "./stream";

export interface AssembleMetricsArgs {
  snapshot?: ContextSnapshot;
  contextAssemblyMs?: number;
  contextTokens?: number;
  outcome: StreamOutcome;
  selection?: ModelSelection;
}

export function assembleMetrics(args: AssembleMetricsArgs): ResponseMetrics {
  const { snapshot, outcome } = args;
  const selection = args.selection ?? outcome.selection;
  const metrics: ResponseMetrics = {};

  if (selection) {
    metrics.provider = selection.providerId;
    metrics.model = selection.model;
  }
  const timings = snapshot?.timings;
  if (timings?.capture !== undefined) metrics.captureMs = timings.capture;
  if (timings?.ocr !== undefined) metrics.ocrMs = timings.ocr;
  if (timings?.accessibility !== undefined) metrics.accessibilityMs = timings.accessibility;
  if (args.contextAssemblyMs !== undefined) metrics.contextAssemblyMs = args.contextAssemblyMs;
  if (outcome.timeToFirstTokenMs !== undefined) metrics.timeToFirstTokenMs = outcome.timeToFirstTokenMs;
  if (outcome.totalMs !== undefined) metrics.totalMs = outcome.totalMs;
  if (outcome.inputTokens !== undefined) metrics.inputTokens = outcome.inputTokens;
  if (outcome.outputTokens !== undefined) metrics.outputTokens = outcome.outputTokens;
  if (args.contextTokens !== undefined) metrics.contextTokens = args.contextTokens;

  return metrics;
}

export type MetricsBus = Pick<typeof eventBus, "emit">;

/** Emit a `dev.metrics` event locally (never crosses into Rust). */
export function emitDevMetrics(
  metrics: ResponseMetrics,
  bus: MetricsBus = eventBus,
  now: () => Date = () => new Date(),
): void {
  const payload: LatencyMetrics = {
    captureMs: metrics.captureMs,
    ocrMs: metrics.ocrMs,
    accessibilityMs: metrics.accessibilityMs,
    contextAssemblyMs: metrics.contextAssemblyMs,
    timeToFirstTokenMs: metrics.timeToFirstTokenMs,
    totalResponseMs: metrics.totalMs,
    inputTokens: metrics.inputTokens,
    outputTokens: metrics.outputTokens,
    updatedAt: now().toISOString(),
  };
  bus.emit("dev.metrics", payload);
}
