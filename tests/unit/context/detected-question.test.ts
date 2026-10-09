/**
 * A live-detected question (proactive loop, no typed instruction) must reach
 * the prompt as its own section — the one the detected_event task line points
 * at — instead of being one transcript line among several recent questions.
 */

import { PromptBuilder } from "@/ai/prompt-builder";
import { allocateBudget } from "@/context/budget";
import { fuseContext } from "@/context/fusion";
import type { DetectedEvent } from "@/lib/types";
import { makeMode, makeSegment, makeSnapshot } from "../../fixtures/helpers/builders";

const segments = [
  makeSegment({
    id: "a",
    speaker: "Interviewer",
    text: "Thanks for joining us today.",
    startTime: 0,
    endTime: 2_000,
  }),
  makeSegment({
    id: "b",
    speaker: "Interviewer",
    text: "Can you walk me through a project you led?",
    startTime: 3_000,
    endTime: 6_000,
  }),
  makeSegment({
    id: "c",
    speaker: "You",
    source: "microphone",
    text: "Sure, I led the migration to Kafka.",
    startTime: 7_000,
    endTime: 12_000,
  }),
  makeSegment({
    id: "d",
    speaker: "Interviewer",
    text: "How did you handle exactly-once delivery?",
    startTime: 13_000,
    endTime: 16_000,
  }),
];

const event: DetectedEvent = {
  id: "evt_42",
  type: "question",
  confidence: 0.92,
  requiresResponse: true,
  text: "How did you handle exactly-once delivery?",
  segmentIds: ["d"],
  speaker: "Interviewer",
  detectedAt: "2026-09-07T09:00:16.000Z",
};

const snapshot = makeSnapshot({ transcript: { segments, windowSeconds: 180 } });

describe("detected question", () => {
  it("becomes exactly one detected_question item and is not repeated in the transcript", () => {
    const items = fuseContext(snapshot, { detectedEvent: event });

    const questions = items.filter((item) => item.source === "detected_question");
    expect(questions).toEqual([
      expect.objectContaining({
        content: "Interviewer: How did you handle exactly-once delivery?",
        relevance: 1,
        ref: "event:evt_42",
      }),
    ]);
    expect(items[0]?.source).toBe("detected_question");
    expect(items.some((item) => item.ref === "segment:d")).toBe(false);
    expect(items.filter((item) => item.source === "transcript")).toHaveLength(3);
    expect(items.some((item) => item.source === "user_instruction")).toBe(false);
  });

  it("falls back to 'Speaker' when the event has no speaker", () => {
    const items = fuseContext(snapshot, { detectedEvent: { ...event, speaker: undefined } });
    expect(items[0]?.content).toBe("Speaker: How did you handle exactly-once delivery?");
  });

  it("yields to a typed instruction", () => {
    const items = fuseContext(snapshot, { instruction: "Summarize the call", detectedEvent: event });
    expect(items.some((item) => item.source === "detected_question")).toBe(false);
    expect(items[0]?.source).toBe("user_instruction");
    expect(items.some((item) => item.ref === "segment:d")).toBe(true);
  });

  it("is kept even when the budget is tiny", () => {
    const items = fuseContext(snapshot, { detectedEvent: event });
    const { included } = allocateBudget(items, 5);
    expect(included.map((item) => item.source)).toContain("detected_question");
  });

  it("renders as the section the detected_event task line points at", () => {
    const items = fuseContext(snapshot, { detectedEvent: event });
    const builder = new PromptBuilder({
      mode: makeMode({ name: "Interview" }),
      style: { length: "concise", tone: "natural" },
      schemaId: "answer",
      trigger: "detected_event",
      items: allocateBudget(items, 4_000).included,
      detectedEvent: event,
      nonce: "n0",
    });
    const context = builder.renderContext();
    const heading = '<context source="Question just asked (heard; may be mis-transcribed)" id="n0">';
    expect(context).toContain(`${heading}\nInterviewer: How did you handle exactly-once delivery?`);
    expect(context.indexOf(heading)).toBeLessThan(context.indexOf('source="Recent conversation'));
    expect(context).not.toContain("Current question");
    expect(builder.renderTask()).toContain('(see "Question just asked")');
  });
});
