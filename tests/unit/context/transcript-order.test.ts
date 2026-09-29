/**
 * The conversation reads in the order it was spoken. Relevance still decides
 * what fits the budget (oldest turns go first); it never decides the order.
 */

import { PromptBuilder } from "@/ai/prompt-builder";
import { allocateBudget } from "@/context/budget";
import { estimateTokens, fuseContext } from "@/context/fusion";
import type { ContextItem } from "@/lib/types";
import { makeMode, makeSegment, makeSnapshot } from "../../fixtures/helpers/builders";

/** Twelve turns 10 s apart, questions from the interviewer and answers from me. */
const turns = Array.from({ length: 12 }, (_, i) =>
  makeSegment({
    id: `t${i}`,
    speaker: i % 2 === 0 ? "Interviewer" : "You",
    source: i % 2 === 0 ? "system" : "microphone",
    text:
      i % 2 === 0
        ? `Turn ${i}: what was the hardest bug you fixed?`
        : `Turn ${i}: it was a race in the cache layer.`,
    startTime: i * 10_000,
    endTime: i * 10_000 + 8_000,
  }),
);

function render(items: ContextItem[]): string {
  return new PromptBuilder({
    mode: makeMode(),
    style: { length: "concise", tone: "natural" },
    schemaId: "answer",
    trigger: "shortcut_generate",
    items,
  }).renderContext();
}

function sectionLines(context: string, heading: string): string[] {
  const start = context.indexOf(`<context source="${heading}"`);
  if (start < 0) return [];
  const body = context.slice(start).split("\n\n")[0] ?? "";
  // Drop the opening and the closing tag of the block.
  return body.split("\n").slice(1, -1);
}

describe("transcript render order", () => {
  it("renders every turn chronologically even though fusion ranks questions first", () => {
    const items = fuseContext(makeSnapshot({ transcript: { segments: turns, windowSeconds: 180 } }));
    // Fusion order is by relevance (questions hoisted), not by time.
    expect(items.map((item) => item.ref)).not.toEqual(turns.map((t) => `segment:${t.id}`));

    const lines = sectionLines(
      render(allocateBudget(items, 4_000).included),
      "Recent conversation (You / Speaker)",
    );
    expect(lines).toEqual(turns.map((t) => `${t.speaker}: ${t.text}`));
  });

  it("drops the oldest turns first under a small budget and keeps the survivors chronological", () => {
    const items = fuseContext(makeSnapshot({ transcript: { segments: turns, windowSeconds: 180 } }));
    const perTurn = estimateTokens(`You: ${turns[1]!.text}`);
    const { included, dropped } = allocateBudget(items, perTurn * 5);

    expect(dropped.map((i) => i.ref)).toContain("segment:t0");
    const lines = sectionLines(render(included), "Recent conversation (You / Speaker)");
    const order = lines.map((line) => Number(/Turn (\d+)/.exec(line)?.[1]));
    expect(order).toEqual([...order].sort((a, b) => a - b));
    expect(order.length).toBeGreaterThan(2);
    expect(order).not.toContain(0);
  });

  it("puts older conversation right before the recent turns, earlier summary first", () => {
    const old = [
      makeSegment({
        id: "o1",
        speaker: "Interviewer",
        text: "Tell me about your background.",
        startTime: 0,
        endTime: 4_000,
      }),
      makeSegment({
        id: "o2",
        speaker: "You",
        source: "microphone",
        text: "I spent six years on payments.",
        startTime: 5_000,
        endTime: 9_000,
      }),
    ];
    const fresh = makeSegment({
      id: "n1",
      speaker: "Interviewer",
      text: "Why did you leave?",
      startTime: 300_000,
      endTime: 303_000,
    });
    const snapshot = makeSnapshot({
      transcript: { segments: [...old, fresh], earlierSummary: "Intro small talk.", windowSeconds: 180 },
      session: {
        sessionId: "ses_1",
        modeId: "general",
        startedAt: "2026-09-07T08:55:00.000Z",
        recentResponses: [{ id: "r1", content: "Earlier answer.", createdAt: "2026-09-07T08:58:00.000Z" }],
        recentEvents: [],
        notes: [],
        documentIds: [],
      },
    });
    const context = render(allocateBudget(fuseContext(snapshot), 4_000).included);

    const earlier = context.indexOf('source="Earlier conversation"');
    const recent = context.indexOf('source="Recent conversation');
    expect(earlier).toBeGreaterThan(-1);
    expect(earlier).toBeLessThan(recent);
    expect(context.indexOf('source="Earlier in this session"')).toBeGreaterThan(recent);
    expect(sectionLines(context, "Earlier conversation")).toEqual([
      "Earlier (summary): Intro small talk.",
      "Interviewer: Tell me about your background.",
      "You: I spent six years on payments.",
    ]);
  });
});
