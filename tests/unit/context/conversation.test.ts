/**
 * Follow-ups build on the chat thread itself — with or without an active
 * session — so "rewrite it in Go" sees the question, the answer and the code
 * it refers to.
 */

import { PromptBuilder } from "@/ai/prompt-builder";
import { allocateBudget } from "@/context/budget";
import { fuseContext } from "@/context/fusion";
import { enrichSnapshot } from "@/context/snapshot";
import type { ContextSnapshot } from "@/lib/types";
import { makeMode, makeResponse, makeSession, makeSettings, makeSnapshot } from "../../fixtures/helpers/builders";

const settings = makeSettings();
const mode = makeMode();

const twoSum = makeResponse({
  id: "resp_two_sum",
  prompt: "Solve Two Sum",
  title: "Two Sum with a hash map",
  content: "Walk the array once and keep a map from value to index; each lookup is O(1), so the whole pass is O(n).",
  code: {
    language: "python",
    code: "def two_sum(nums, target):\n    seen = {}\n    for i, n in enumerate(nums):\n        if target - n in seen:\n            return [seen[target - n], i]\n        seen[n] = i",
  },
  createdAt: "2026-09-07T09:01:00.000Z",
});

const older = [1, 2, 3].map((n) =>
  makeResponse({
    id: `resp_old_${n}`,
    prompt: `Old question ${n}`,
    title: `Old answer ${n}`,
    content: `Old answer body ${n} `.repeat(40),
    createdAt: `2026-09-07T08:5${n}:00.000Z`,
  }),
);

function prompt(snapshot: ContextSnapshot, instruction: string, trigger: "follow_up" | "typed" = "follow_up") {
  const items = fuseContext(snapshot, { instruction });
  return new PromptBuilder({
    mode,
    style: { length: "concise", tone: "natural" },
    schemaId: "answer",
    trigger,
    items: allocateBudget(items, 6_000).included,
    instruction,
  });
}

describe("conversation memory", () => {
  it("keeps the chat thread without an active session", () => {
    const enriched = enrichSnapshot(makeSnapshot(), {
      mode,
      settings,
      session: null,
      instruction: "Can you rewrite your solution in Go?",
      previousResponses: [twoSum],
    });
    expect(enriched.session).toBeUndefined();
    expect(enriched.conversation).toEqual([
      expect.objectContaining({ id: "resp_two_sum", prompt: "Solve Two Sum", code: twoSum.code }),
    ]);
  });

  it("renders the previous question, answer and code before the follow-up", () => {
    const instruction = "Can you rewrite your solution in Go?";
    const enriched = enrichSnapshot(makeSnapshot(), { mode, settings, instruction, previousResponses: [twoSum] });
    const builder = prompt(enriched, instruction);
    const context = builder.renderContext();

    const chat = context.indexOf("### Earlier in this chat");
    const question = context.indexOf("### Current question");
    expect(chat).toBeGreaterThan(-1);
    expect(chat).toBeLessThan(question);
    expect(context).toContain("Q: Solve Two Sum\nA: Walk the array once");
    expect(context).toContain("```python\ndef two_sum(nums, target):");
    expect(context).toContain("return [seen[target - n], i]");
    expect(builder.renderTask()).toContain("Answer this follow-up");
  });

  it("keeps the last two turns in full, older ones as short summaries, oldest first", () => {
    const instruction = "and the space complexity?";
    const enriched = enrichSnapshot(makeSnapshot(), {
      mode,
      settings,
      instruction,
      previousResponses: [...older, twoSum],
    });
    const context = prompt(enriched, instruction).renderContext();
    const start = context.indexOf("### Earlier in this chat");
    const section = context.slice(start, context.indexOf("### Current question"));

    expect(section).toContain("Q: Old question 1\nA (summary): Old answer 1");
    expect(section).toContain("Q: Old question 3\nA: Old answer body 3");
    expect(section.indexOf("Old question 1")).toBeLessThan(section.indexOf("Old question 2"));
    expect(section.indexOf("Old question 3")).toBeLessThan(section.indexOf("Solve Two Sum"));
    // Only the newest turn carries its code.
    expect(section.match(/```/g)).toHaveLength(2);
  });

  it("takes the code from a fenced block when the answer has no code field", () => {
    const plain = makeResponse({
      id: "resp_plain",
      prompt: "Reverse a list in JS",
      content: "Use the built-in:\n\n```js\nconst reversed = [...xs].reverse();\n```\n\nIt copies first so the input stays intact.",
      createdAt: "2026-09-07T09:02:00.000Z",
    });
    const instruction = "why copy first?";
    const enriched = enrichSnapshot(makeSnapshot(), { mode, settings, instruction, previousResponses: [plain] });
    const context = prompt(enriched, instruction).renderContext();
    expect(context).toContain("```js\nconst reversed = [...xs].reverse();\n```");
    expect(context).toContain("It copies first so the input stays intact.");
  });

  it("does not repeat chat turns as session memory", () => {
    const instruction = "shorter please";
    const enriched = enrichSnapshot(makeSnapshot(), {
      mode,
      settings,
      session: makeSession(),
      instruction,
      previousResponses: [twoSum],
    });
    const items = fuseContext(enriched, { instruction });
    expect(items.filter((item) => item.ref === "response:resp_two_sum").map((item) => item.source)).toEqual([
      "conversation",
    ]);
  });

  it("keeps the responses the native builder loaded when the UI passes none", () => {
    const native = makeSnapshot({
      session: {
        sessionId: "ses_1",
        modeId: "general",
        startedAt: "2026-09-07T08:55:00.000Z",
        recentResponses: [{ id: "r_db", content: "Answer loaded from the DB.", createdAt: "2026-09-07T08:57:00.000Z" }],
        recentEvents: [],
        notes: [],
        documentIds: [],
      },
    });
    const enriched = enrichSnapshot(native, { mode, settings, session: makeSession() });
    expect(enriched.session?.recentResponses.map((r) => r.id)).toEqual(["r_db"]);
  });

  it("uses the typed task line for a follow-up with nothing to follow", () => {
    const instruction = "why not B?";
    const enriched = enrichSnapshot(makeSnapshot(), { mode, settings, instruction });
    const task = prompt(enriched, instruction).renderTask();
    expect(task).toContain("Answer my question below");
    expect(task).not.toContain("follow-up");
  });
});
