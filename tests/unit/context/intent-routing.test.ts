/**
 * classifyIntent routing: which asks get the full-solution coding schema
 * (MODE-001), the debug shape, and what spoken triggers do in technical
 * modes (MODE-004).
 */

import { classifyIntent, type Intent } from "@/context/relevance";
import type { AskTrigger } from "@/lib/engine-contract";
import type { BlueyMode, DetectedEvent } from "@/lib/types";
import { makeMode, makeSnapshot } from "../../fixtures/helpers/builders";

const NOW = () => new Date("2026-09-07T09:05:00.000Z");
const EVENT: DetectedEvent = {
  id: "evt_1",
  type: "question",
  confidence: 0.9,
  requiresResponse: true,
  text: "",
  segmentIds: ["seg_1"],
  speaker: "Interviewer",
  detectedAt: NOW().toISOString(),
};

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

describe("technical modes answer non-technical questions as speech (MODE-004)", () => {
  const codingMode = makeMode({ id: "coding-interview", responseSchema: "coding" });
  const designMode = makeMode({ id: "system-design", responseSchema: "system-design" });
  const heard = (text: string, type: DetectedEvent["type"] = "question") =>
    classifyIntent({
      snapshot: makeSnapshot(),
      mode: codingMode,
      detectedEvent: { ...EVENT, type, text },
      trigger: "detected_event",
      now: NOW,
    });

  it("answers an intro question in Coding Interview as speech on the suggestion schema", () => {
    const intent = heard("Tell me about yourself.");
    expect(intent.task).toBe("answer");
    expect(intent.schemaId).toBe("suggested-response");
    expect(intent.answerShape).toBe("spoken");
  });

  it("uses the behavioral schema for a behavioral question", () => {
    const intent = heard("Tell me about a time you disagreed with a teammate.", "behavioral_question");
    expect(intent.schemaId).toBe("behavioral");
    expect(intent.answerShape).toBe("spoken");
  });

  it("keeps coding and design asks on their schemas", () => {
    expect(heard("Can you implement two-sum for me?", "coding_problem").schemaId).toBe("coding");
    const design = classifyIntent({
      snapshot: makeSnapshot(),
      mode: designMode,
      detectedEvent: { ...EVENT, text: "How would you design a URL shortener?" },
      trigger: "detected_event",
      now: NOW,
    });
    expect(design.task).toBe("system_design");
    expect(design.schemaId).toBe("system-design");
  });

  it("answers a typed non-technical question in System Design on the answer schema", () => {
    const intent = classifyIntent({
      snapshot: makeSnapshot(),
      mode: designMode,
      instruction: "What does the acronym CAP stand for?",
      trigger: "typed",
      now: NOW,
    });
    expect(intent.task).toBe("answer");
    expect(intent.schemaId).toBe("answer");
  });

  it("still solves on ⌘↵ in Coding Interview when code is on screen", () => {
    expect(intentFor({ screen: IDE_CODE, mode: codingMode }).schemaId).toBe("coding");
  });
});

describe("the voice follows the deliverable (MODE-002)", () => {
  const interview = makeMode({ id: "interview", responseSchema: "suggested-response" });
  const lecture = makeMode({ id: "lecture", responseSchema: "lecture" });
  const ask = (instruction: string | undefined, mode: BlueyMode, trigger: AskTrigger) =>
    classifyIntent({ snapshot: makeSnapshot(), mode, instruction, trigger, now: NOW });

  it("explains to me when I type an explanation request in a conversational mode", () => {
    const intent = ask("explain what a mutex is so I understand it", interview, "typed");
    expect(intent.answerShape).toBe("explain");
    expect(intent.voice).toBe("explain-to-user");
  });

  it("keeps a typed interview question addressed to me as words to say", () => {
    const intent = ask("Why do you want to leave your current role?", interview, "typed");
    expect(intent.answerShape).toBe("spoken");
    expect(intent.voice).toBe("speak-as-user");
  });

  it("speaks as me on ⌘⇧↵", () => {
    expect(ask(undefined, interview, "shortcut_generate").voice).toBe("speak-as-user");
  });

  it("writes as me for a pick or a text to send", () => {
    expect(ask("Which of the following is true about TCP?", makeMode(), "typed").voice).toBe("write-as-user");
    expect(ask("Write a reply to this email declining politely", makeMode(), "typed").voice).toBe(
      "write-as-user",
    );
  });

  it("explains to me for a lecture recap or a why question", () => {
    expect(ask("Summarize the last five minutes", lecture, "typed").voice).toBe("explain-to-user");
    expect(ask("Why does TCP need a three-way handshake?", makeMode(), "typed").voice).toBe(
      "explain-to-user",
    );
  });
});
