import { describe, expect, it } from "vitest";

import { derivePill } from "@/features/hud/state-pill";
import { buildTranscriptLines } from "@/features/hud/transcript-strip";
import type { AppStatus, BlueyError, DetectedEvent } from "@/lib/types";
import { makeSegment } from "./helpers";

function status(partial: Partial<AppStatus>): AppStatus {
  return {
    state: "ready",
    audioActive: false,
    modeId: "general",
    updatedAt: new Date().toISOString(),
    ...partial,
  };
}

const error: BlueyError = {
  kind: "configuration",
  code: "config.missing_provider",
  message: "No provider",
  recoverable: true,
  recovery: { type: "open_settings", tab: "ai" },
};

describe("derivePill", () => {
  it("carries the error so the pill can offer its recovery", () => {
    expect(derivePill(status({ state: "error", error }), null, false, "General")).toEqual({
      kind: "error",
      error,
    });
  });

  it("orders error > busy > prepared > preparing > listening > idle", () => {
    expect(derivePill(status({ state: "error", error }), "thinking", true, "General", true).kind).toBe(
      "error",
    );
    expect(derivePill(status({}), "streaming", true, "General", true).kind).toBe("thinking");
    expect(derivePill(status({ audioActive: true }), null, true, "General", true).kind).toBe("prepared");
    expect(derivePill(status({ audioActive: true }), null, false, "General", true).kind).toBe("preparing");
    expect(derivePill(status({ audioActive: true }), null, false, "General").kind).toBe("listening");
    expect(derivePill(status({}), "done", false, "Sales")).toEqual({ kind: "idle", modeName: "Sales" });
  });
});

describe("buildTranscriptLines", () => {
  it("keeps the newest finals, appends the partial and flags detected segments", () => {
    const a = makeSegment({ text: "First" });
    const b = makeSegment({
      text: "Second",
      source: "microphone",
      speaker: undefined,
      speakerConfidence: undefined,
    });
    const c = makeSegment({ text: "Third question?" });
    const partial = makeSegment({ text: "typing", finalized: false });
    const question: DetectedEvent = {
      id: "det-c",
      type: "question",
      confidence: 0.9,
      requiresResponse: true,
      text: c.text,
      segmentIds: [c.id],
      detectedAt: new Date().toISOString(),
    };

    const lines = buildTranscriptLines([a, b, c], partial, [question], undefined, 3);
    expect(lines.map((l) => l.text)).toEqual(["Second", "Third question?", "typing"]);
    expect(lines[0]?.speaker).toBe("You");
    expect(lines[0]?.speakerConfidence).toBeCloseTo(0.95);
    expect(lines[1]?.detected).toBe(true);
    expect(lines[2]?.partial).toBe(true);
    expect(lines[0]?.detected).toBe(false);
  });

  it("collapses to the single most recent line (the partial wins when present)", () => {
    const a = makeSegment({ text: "Older" });
    const b = makeSegment({ text: "Newest" });
    const partial = makeSegment({ text: "typing", finalized: false });
    expect(buildTranscriptLines([a, b], null, [], undefined, 1).map((l) => l.text)).toEqual(["Newest"]);
    expect(buildTranscriptLines([a, b], partial, [], undefined, 1).map((l) => l.text)).toEqual(["typing"]);
    expect(buildTranscriptLines([], null, [], undefined, 1)).toEqual([]);
  });
});
