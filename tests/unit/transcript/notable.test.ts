import { describe, expect, it } from "vitest";

import type { DetectedEvent } from "@/lib/types";
import { classifySegment } from "@/transcript/classifier";
import { notableEntry } from "@/transcript/notable";
import { makeMode, makeSegment } from "../../fixtures/helpers/builders";

function detection(patch: Partial<DetectedEvent> = {}): DetectedEvent {
  return {
    id: "det-1",
    type: "decision",
    confidence: 0.86,
    requiresResponse: false,
    text: "We decided to ship on Friday.",
    segmentIds: ["seg-1"],
    detectedAt: "2026-09-28T10:00:00.000Z",
    ...patch,
  };
}

describe("notableEntry (MODE-006)", () => {
  const meeting = makeMode({ id: "team-meeting", responseSchema: "meeting" });
  const lecture = makeMode({ id: "lecture", responseSchema: "lecture" });

  it("turns a Team Meeting decision into a decision_detected timeline entry", () => {
    expect(notableEntry(detection(), meeting, true)).toEqual({
      type: "decision_detected",
      title: "Decision",
      detail: "We decided to ship on Friday.",
      refs: { segmentId: "seg-1" },
      confidence: 0.86,
    });
  });

  it("maps what the classifier really detects in a meeting", () => {
    const segment = makeSegment({ text: "Action item: Sam will send the rollout plan by Monday." });
    const event = classifySegment({ segment, recent: [], mode: meeting });
    expect(event && notableEntry(event, meeting, true)?.type).toBe("action_item_detected");
  });

  it("keeps only the kinds each mode advertises", () => {
    expect(notableEntry(detection({ type: "important_statement" }), lecture, true)?.type).toBe(
      "important_statement",
    );
    expect(notableEntry(detection(), lecture, true)).toBeNull();
    expect(notableEntry(detection(), makeMode({ responseSchema: "behavioral" }), true)).toBeNull();
  });

  it("skips detections that already get an answer", () => {
    expect(notableEntry(detection({ type: "question", requiresResponse: true }), meeting, true)).toBeNull();
    expect(notableEntry(detection({ requiresResponse: true }), meeting, true)).toBeNull();
  });

  it("stores no spoken words when transcripts are not kept", () => {
    const entry = notableEntry(detection(), meeting, false);
    expect(entry?.type).toBe("decision_detected");
    expect(entry).not.toHaveProperty("detail");
  });
});
