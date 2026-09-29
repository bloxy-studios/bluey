/**
 * classifyIntent routing: which asks get the full-solution coding schema
 * (MODE-001), the debug shape, and what spoken triggers do in technical
 * modes (MODE-004).
 */

import { classifyIntent, type Intent } from "@/context/relevance";
import type { AskTrigger } from "@/lib/engine-contract";
import type { BlueyMode } from "@/lib/types";
import { makeMode, makeSnapshot } from "../../fixtures/helpers/builders";

const NOW = () => new Date("2026-09-07T09:05:00.000Z");

function intentFor(opts: {
  screen: string;
  instruction?: string;
  trigger?: AskTrigger;
  mode?: BlueyMode;
}): Intent {
  return classifyIntent({
    snapshot: makeSnapshot({
      screen: { width: 1600, height: 1000, frameId: "f1" },
      ocr: { blocks: [], text: opts.screen, level: "fast", languages: ["en"], durationMs: 1 },
    }),
    instruction: opts.instruction,
    trigger: opts.trigger ?? (opts.instruction ? "typed" : "shortcut_capture"),
    mode: opts.mode ?? makeMode(),
    now: NOW,
  });
}

const IDE_CODE = [
  "export async function loadUser(id: string) {",
  "  const res = fetch(`/api/users/${id}`);",
  "  const body = res.json();",
  "  return body.user;",
  "}",
].join("\n");

const LEETCODE = [
  "1. Two Sum",
  "Given an array of integers nums and an integer target, return indices of the two numbers such that they add up to target.",
  "Example 1:",
  "Input: nums = [2,7,11,15], target = 9",
  "Output: [0,1]",
  "Constraints:",
  "2 <= nums.length <= 10^4",
].join("\n");

const TYPE_ERROR = "TypeError: res.json is not a function\n    at loadUser (user.ts:3:20)";

describe("coding schema only for problems and solve requests (MODE-001)", () => {
  it("explains code on screen instead of re-solving it", () => {
    const intent = intentFor({ screen: IDE_CODE, instruction: "what does this function do?" });
    expect(intent.task).toBe("answer");
    expect(intent.schemaId).toBe("answer");
  });

  it("does not treat prose that mentions a class as code", () => {
    const news = "Rising rents squeeze the middle class in every major city, the report finds. ".repeat(3);
    expect(intentFor({ screen: news, instruction: "Summarise the argument in one line" }).task).not.toBe(
      "coding",
    );
    expect(intentFor({ screen: news, instruction: "Who is most affected?" }).task).toBe("answer");
  });

  it("still solves a problem statement, typed 'solve this' or ⌘↵", () => {
    for (const instruction of ["solve this", "what's the answer?", undefined]) {
      const intent = intentFor({ screen: LEETCODE, instruction });
      expect(intent.task, String(instruction)).toBe("coding");
      expect(intent.schemaId, String(instruction)).toBe("coding");
      expect(intent.answerShape, String(instruction)).toBe("code");
    }
  });

  it("gives code plus an error the debug shape on the answer schema", () => {
    const intent = intentFor({ screen: `${IDE_CODE}\n${TYPE_ERROR}` });
    expect(intent.answerShape).toBe("debug");
    expect(intent.schemaId).toBe("answer");
  });

  it("routes 'why is this failing' over code to the debug shape", () => {
    const intent = intentFor({ screen: IDE_CODE, instruction: "why is this failing?" });
    expect(intent.answerShape).toBe("debug");
    expect(intent.schemaId).toBe("answer");
  });
});
