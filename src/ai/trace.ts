/**
 * The ⌘↵ fast-path trace, pure (ADR 0010 §2, `docs/LATENCY.md`): cumulative
 * stage milestones of a `LatencyTrace`, nearest-rank percentiles and the
 * per-stage summary the dev overlay shows. Mirrors `bluey_core::latency`.
 */

import type { BenchRow, BenchStage, LatencyTrace } from "@/lib/types";

/** Traces the dev overlay keeps for its percentiles. */
export const OVERLAY_WINDOW = 50;

export interface StageDef {
  id: BenchStage;
  label: string;
  key: keyof LatencyTrace;
}

/** The milestones of the `docs/LATENCY.md` table, in order. */
export const STAGES: readonly StageDef[] = [
  { id: "capture", label: "capture", key: "tCaptureDone" },
  { id: "snapshot_ready", label: "snapshot ready", key: "tSnapshotReady" },
  { id: "retrieval", label: "retrieval", key: "tRetrievalDone" },
  { id: "prompt_built", label: "prompt built", key: "tPromptBuilt" },
  { id: "request_sent", label: "request sent (local total)", key: "tRequestSent" },
  { id: "response_headers", label: "response headers", key: "tResponseHeaders" },
  { id: "first_token", label: "first token", key: "tFirstToken" },
  { id: "first_paint", label: "first paint", key: "tFirstPaint" },
  { id: "done", label: "done", key: "tDone" },
] as const;

function stamp(trace: LatencyTrace, key: keyof LatencyTrace): number | undefined {
  const value = trace[key];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

/** Where the trace starts: the trigger, else the earliest stamp it has. */
export function origin(trace: LatencyTrace): number | undefined {
  if (trace.tShortcut !== undefined) return trace.tShortcut;
  let min: number | undefined;
  for (const stage of STAGES) {
    const value = stamp(trace, stage.key);
    if (value !== undefined && (min === undefined || value < min)) min = value;
  }
  return min;
}

/** Milliseconds from the origin to `stage` (cumulative), when both exist. */
export function milestone(trace: LatencyTrace, stage: BenchStage): number | undefined {
  const def = STAGES.find((s) => s.id === stage);
  if (!def) return undefined;
  const from = origin(trace);
  const at = stamp(trace, def.key);
  if (from === undefined || at === undefined || at < from) return undefined;
  return at - from;
}

/** Nearest-rank percentile of `values` (any order; `q` in 0..1). Never the mean. */
export function percentile(values: readonly number[], q: number): number | undefined {
  const sorted = values.filter((v) => Number.isFinite(v)).sort((a, b) => a - b);
  if (sorted.length === 0) return undefined;
  const clamped = Math.min(1, Math.max(0, q));
  const rank = Math.max(1, Math.ceil(clamped * sorted.length));
  return sorted[Math.min(rank, sorted.length) - 1];
}

/** One row per stage that at least one trace reached (cumulative ms since the trigger). */
export function summarize(traces: readonly LatencyTrace[]): BenchRow[] {
  const rows: BenchRow[] = [];
  for (const stage of STAGES) {
    const values: number[] = [];
    for (const trace of traces) {
      const ms = milestone(trace, stage.id);
      if (ms !== undefined) values.push(ms);
    }
    if (values.length === 0) continue;
    rows.push({
      stage: stage.id,
      label: stage.label,
      samples: values.length,
      p50Ms: percentile(values, 0.5),
      p95Ms: percentile(values, 0.95),
    });
  }
  return rows;
}

/** Keep the newest `OVERLAY_WINDOW` traces, one per request id. */
export function pushTrace(traces: readonly LatencyTrace[], trace: LatencyTrace, window = OVERLAY_WINDOW): LatencyTrace[] {
  const next = traces.filter((t) => t.requestId !== trace.requestId);
  next.push(trace);
  return next.length > window ? next.slice(next.length - window) : next;
}

/** `12.3 ms` under 100 ms, `250 ms` above, `—` when absent. */
export function formatStageMs(value: number | undefined): string {
  if (value === undefined || !Number.isFinite(value)) return "—";
  return value >= 100 ? `${value.toFixed(0)} ms` : `${value.toFixed(1)} ms`;
}

/**
 * The WebView clock: `performance.now()` where it exists (test doubles may not
 * have it), `Date.now()` otherwise. Only ever used for differences.
 */
export function perfNow(): number {
  return typeof performance !== "undefined" && typeof performance.now === "function"
    ? performance.now()
    : Date.now();
}

/** Run `callback` after the next paint (falls back to a macrotask without a DOM). */
export function afterNextPaint(callback: () => void): void {
  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => callback());
  } else {
    setTimeout(callback, 0);
  }
}
