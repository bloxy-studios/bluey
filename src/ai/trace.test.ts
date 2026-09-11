import { describe, expect, it } from "vitest";

import type { LatencyTrace } from "@/lib/types";
import { formatStageMs, milestone, origin, percentile, pushTrace, summarize } from "./trace";

function trace(id: string, overrides: Partial<LatencyTrace> = {}): LatencyTrace {
  return {
    requestId: id,
    trigger: "shortcut_capture",
    tShortcut: 900,
    tCaptureDone: 980,
    tSnapshotReady: 1012,
    tRetrievalDone: 1020,
    tPromptBuilt: 1032,
    tRequestSent: 1045,
    tResponseHeaders: 1100,
    tFirstToken: 1400,
    tFirstPaint: 1417,
    tDone: 2022,
    imageBytes: 180_000,
    imagePx: 1440,
    promptTokens: 2100,
    providerId: "gemini",
    model: "gemini-3.5-flash-lite",
    ...overrides,
  };
}

describe("fast-path trace math", () => {
  it("measures cumulative milestones from the trigger", () => {
    const t = trace("req_1");
    expect(milestone(t, "capture")).toBe(80);
    expect(milestone(t, "snapshot_ready")).toBe(112);
    expect(milestone(t, "request_sent")).toBe(145);
    expect(milestone(t, "first_token")).toBe(500);
    expect(milestone(t, "first_paint")).toBe(517);
    expect(milestone(t, "done")).toBe(1122);
    expect(milestone(trace("req_2", { tFirstPaint: undefined }), "first_paint")).toBeUndefined();
    expect(milestone(trace("req_3", { tCaptureDone: 10 }), "capture")).toBeUndefined();
  });

  it("falls back to the earliest stamp when there is no trigger moment", () => {
    const internal = trace("req_4", {
      trigger: "internal",
      tShortcut: undefined,
      tCaptureDone: undefined,
      tSnapshotReady: undefined,
      tRetrievalDone: undefined,
      tPromptBuilt: undefined,
      tFirstPaint: undefined,
    });
    expect(origin(internal)).toBe(1045);
    expect(milestone(internal, "request_sent")).toBe(0);
    expect(milestone(internal, "first_token")).toBe(355);
    expect(origin({ requestId: "empty", trigger: "internal" })).toBeUndefined();
  });

  it("uses nearest-rank percentiles, never the mean", () => {
    expect(percentile([10, 20, 30, 40, 100], 0.5)).toBe(30);
    expect(percentile([10, 20, 30, 40, 100], 0.95)).toBe(100);
    expect(percentile([10, 20, 30, 40, 100], 0)).toBe(10);
    expect(percentile([7], 0.95)).toBe(7);
    expect(percentile([], 0.5)).toBeUndefined();
    expect(percentile([Number.NaN, 3], 0.5)).toBe(3);
    expect(percentile([5, 1, 3, 2, 4, 6, 8, 7, 9, 10], 0.5)).toBe(5);
    expect(percentile([5, 1, 3, 2, 4, 6, 8, 7, 9, 10], 0.9)).toBe(9);
  });

  it("summarizes a window of traces per stage", () => {
    const traces = [0, 1, 2, 3, 4].map((i) => trace(`req_${i}`, { tRequestSent: 1045 + i * 10, tFirstPaint: undefined }));
    const rows = summarize(traces);
    const local = rows.find((r) => r.stage === "request_sent");
    expect(local).toMatchObject({ samples: 5, p50Ms: 165, p95Ms: 185, label: "request sent (local total)" });
    expect(rows.find((r) => r.stage === "capture")).toMatchObject({ samples: 5, p50Ms: 80 });
    expect(rows.find((r) => r.stage === "first_paint")).toBeUndefined();
    expect(rows.map((r) => r.stage)).toEqual([
      "capture",
      "snapshot_ready",
      "retrieval",
      "prompt_built",
      "request_sent",
      "response_headers",
      "first_token",
      "done",
    ]);
    expect(summarize([])).toEqual([]);
  });

  it("keeps the newest traces, one per request", () => {
    let window: LatencyTrace[] = [];
    for (let i = 0; i < 60; i += 1) window = pushTrace(window, trace(`req_${i}`));
    expect(window).toHaveLength(50);
    expect(window[0]?.requestId).toBe("req_10");
    const updated = pushTrace(window, trace("req_59", { tFirstPaint: 1500 }));
    expect(updated).toHaveLength(50);
    expect(updated.filter((t) => t.requestId === "req_59")).toHaveLength(1);
    expect(updated.at(-1)?.tFirstPaint).toBe(1500);
  });

  it("formats stage durations like the bench table", () => {
    expect(formatStageMs(12.345)).toBe("12.3 ms");
    expect(formatStageMs(250.4)).toBe("250 ms");
    expect(formatStageMs(undefined)).toBe("—");
  });
});
