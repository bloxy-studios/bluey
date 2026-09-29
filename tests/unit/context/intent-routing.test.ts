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

  it("does not read an exception class named in ordinary code as an error on screen", () => {
    const python = [
      "def parse_age(value):",
      "    if not value.isdigit():",
      '        raise ValueError("age must be a number")',
      "    return int(value)",
    ].join("\n");
    const java = [
      "class Repo {",
      "  User load(String id) throws IOException {",
      "    try { return db.get(id); } catch (IOException e) { throw e; }",
      "  }",
      "}",
    ].join("\n");
    const codingMode = makeMode({ id: "coding-interview", responseSchema: "coding" });
    for (const screen of [python, java]) {
      for (const mode of [makeMode(), codingMode]) {
        for (const instruction of [undefined, "add type hints to this function"]) {
          const label = `${mode.id} ${String(instruction)} ${screen.slice(0, 12)}`;
          const intent = intentFor({ screen, instruction, mode });
          expect(intent.answerShape, label).not.toBe("debug");
          if (mode === codingMode) expect(intent.schemaId, label).toBe("coding");
        }
      }
    }
  });

  it("still reads an error report line as an error beside the code", () => {
    for (const report of ["ValueError: age must be a number", "java.io.IOException: disk full"]) {
      const intent = intentFor({ screen: `${IDE_CODE}\n${report}` });
      expect(intent.answerShape, report).toBe("debug");
    }
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

  it("keeps a regenerated heard question spoken, on the suggestion schema (UX-011)", () => {
    for (const mode of [makeMode(), codingMode]) {
      const again = classifyIntent({
        snapshot: makeSnapshot(),
        mode,
        detectedEvent: { ...EVENT, text: "What is your experience with Kubernetes?" },
        trigger: "regenerate",
        now: NOW,
      });
      expect(again.answerShape, mode.id).toBe("spoken");
      expect(again.voice, mode.id).toBe("speak-as-user");
      if (mode === codingMode) expect(again.schemaId).toBe("suggested-response");
    }
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
    // The conversational schema's `content` is "exactly what I say": an explanation uses the plain answer.
    expect(intent.schemaId).toBe("answer");
    expect(intent.responseType).toBe("answer");
  });

  it("keeps a typed interview question addressed to me as words to say", () => {
    const intent = ask("Why do you want to leave your current role?", interview, "typed");
    expect(intent.answerShape).toBe("spoken");
    expect(intent.voice).toBe("speak-as-user");
    expect(intent.schemaId).toBe("suggested-response");
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

describe("yes/no only for real yes/no questions (AI-005)", () => {
  const shapeOf = (instruction: string) => intentFor({ screen: "", instruction }).answerShape;

  it("does not force yes or no on an either/or question", () => {
    expect(shapeOf("Should I use Postgres or Mongo for this?")).toBe("short_answer");
    expect(shapeOf("Is the capital Sydney or Canberra?")).toBe("short_answer");
  });

  it("keeps 'or not' and plain yes/no questions boolean", () => {
    expect(shapeOf("Is this endpoint idempotent or not?")).toBe("boolean");
    expect(shapeOf("Does Python pass arguments by reference?")).toBe("boolean");
  });

  it("answers a forecast with a best estimate, not a yes or no", () => {
    expect(shapeOf("Will the Fed cut rates next month?")).toBe("short_answer");
    expect(shapeOf("Is it likely to rain tomorrow?")).toBe("short_answer");
  });
});

describe("eval-matrix routing gaps (TEST-001 rows 13, 23, 25)", () => {
  const interview = makeMode({ id: "interview", responseSchema: "suggested-response" });
  const codingMode = makeMode({ id: "coding-interview", responseSchema: "coding" });

  it("treats a bare stack trace under ⌘↵ as a bug to fix", () => {
    const intent = intentFor({
      screen:
        "TypeError: Cannot read properties of undefined (reading 'map')\n    at renderList (List.tsx:12:18)",
    });
    expect(intent.answerShape).toBe("debug");
    expect(intent.schemaId).toBe("answer");
  });

  it("keeps an error word in prose on ⌘↵ an explanation", () => {
    const intent = intentFor({
      screen: "A NetworkError is raised when the browser cannot reach the server.",
    });
    expect(intent.answerShape).not.toBe("debug");
  });

  it("gives words to say when I ask what to say, even with a why inside", () => {
    const ask = (instruction: string, mode: BlueyMode) =>
      classifyIntent({ snapshot: makeSnapshot(), mode, instruction, trigger: "typed", now: NOW });
    expect(ask("How should I answer why I want to work at Acme?", interview).answerShape).toBe("spoken");
    const exact = ask("Give me the exact words to say to decline the meeting", makeMode());
    expect(exact.answerShape).toBe("spoken");
    expect(exact.voice).toBe("speak-as-user");
  });

  it("continues a code answer as code on a follow-up in a coding mode", () => {
    const snapshot = makeSnapshot({
      conversation: [
        {
          id: "r1",
          prompt: "Solve Two Sum",
          content: "```python\ndef two_sum(nums, target):\n    seen = {}\n```",
          createdAt: NOW().toISOString(),
        },
      ],
    });
    const followUp = (instruction: string) =>
      classifyIntent({ snapshot, mode: codingMode, instruction, trigger: "follow_up", now: NOW });
    expect(followUp("And in Go?").task).toBe("coding");
    expect(followUp("And in Go?").answerShape).toBe("code");
    expect(followUp("Why is that O(n)?").answerShape).toBe("explain");
  });
});
