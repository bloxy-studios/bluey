import { classifyIntent, mentionsExternalInfo, VISION_TEXT_SUFFICIENCY_CHARS } from "@/context/relevance";
import type { DetectedEvent } from "@/lib/types";
import { makeMode, makeSnapshot } from "../../fixtures/helpers/builders";
import { loadAllFixtures } from "../../fixtures/helpers/fixtures";

const NOW = () => new Date("2026-09-07T09:05:00.000Z");

function eventFor(type: DetectedEvent["type"], text: string): DetectedEvent {
  return {
    id: "evt_test",
    type,
    confidence: 0.9,
    requiresResponse: true,
    text,
    segmentIds: ["seg_x"],
    speaker: "Interviewer",
    detectedAt: NOW().toISOString(),
  };
}

describe("classifyIntent per fixture", () => {
  for (const fixture of loadAllFixtures()) {
    it(`matches expected intent for ${fixture.name}`, () => {
      const lastSegment = fixture.transcript[fixture.transcript.length - 1]!;
      const intent = classifyIntent({
        instruction: fixture.expected.instruction,
        snapshot: fixture.snapshot,
        mode: fixture.mode,
        detectedEvent: eventFor(fixture.expected.detectedEventType, lastSegment.text),
        now: NOW,
      });
      expect(intent.task).toBe(fixture.expected.task);
      expect(intent.schemaId).toBe(fixture.expected.schemaId);
      expect(intent.responseType).toBe(fixture.expected.responseType);
      expect(intent.visionRequired).toBe(fixture.expected.visionRequired);
    });
  }
});

describe("classifyIntent specifics", () => {
  it("upgrades a generic mode to coding when a problem is visible on screen", () => {
    const snapshot = makeSnapshot({
      screen: { width: 1600, height: 1000, frameId: "f1" },
      ocr: {
        blocks: [{ text: "x", confidence: 0.95, boundingBox: { x: 0, y: 0, width: 10, height: 10 } }],
        text: [
          "Example 1:",
          "Input: nums = [2,7,11,15], target = 9",
          "Output: [0,1]",
          "Constraints:",
          "2 <= nums.length <= 10^4",
          "function twoSum(nums, target) {",
        ].join("\n"),
        level: "accurate",
        languages: ["en"],
        durationMs: 10,
      },
    });
    const intent = classifyIntent({ snapshot, mode: makeMode(), now: NOW });
    expect(intent.task).toBe("coding");
    expect(intent.schemaId).toBe("coding");
    expect(intent.responseType).toBe("code");
  });

  it("routes summarize asks to summarization with a balanced-or-slower budget", () => {
    const intent = classifyIntent({
      instruction: "Summarize what we covered so far",
      snapshot: makeSnapshot(),
      mode: makeMode({ preferredLatency: "ultra-fast" }),
      now: NOW,
    });
    expect(intent.task).toBe("summarization");
    expect(["balanced", "deep"]).toContain(intent.latency);
  });

  it("uses deep reasoning and deep latency for system design", () => {
    const intent = classifyIntent({
      instruction: "How would you design a rate limiter for our public API?",
      snapshot: makeSnapshot(),
      mode: makeMode(),
      now: NOW,
    });
    expect(intent.task).toBe("system_design");
    expect(intent.reasoning).toBe("deep");
    expect(intent.latency).toBe("deep");
    expect(intent.schemaId).toBe("system-design");
  });

  it("detects research asks about current external info", () => {
    const intent = classifyIntent({
      instruction: "What is the latest news about Anthropic?",
      snapshot: makeSnapshot(),
      mode: makeMode(),
      now: NOW,
    });
    expect(intent.task).toBe("research");
    expect(intent.responseType).toBe("research");
  });

  it("requires vision only when text is insufficient and a frame exists", () => {
    const richText = "a".repeat(VISION_TEXT_SUFFICIENCY_CHARS + 50);
    const base = {
      mode: makeMode(),
      now: NOW,
    };
    const noScreen = classifyIntent({
      ...base,
      instruction: "What is this?",
      snapshot: makeSnapshot(),
    });
    expect(noScreen.visionRequired).toBe(false);

    const sparse = classifyIntent({
      ...base,
      instruction: "What is this?",
      snapshot: makeSnapshot({ screen: { width: 100, height: 100, frameId: "f" }, ocr: { blocks: [], text: "tiny", level: "fast", languages: ["en"], durationMs: 1 } }),
    });
    expect(sparse.visionRequired).toBe(true);

    const rich = classifyIntent({
      ...base,
      instruction: "What does the log say?",
      snapshot: makeSnapshot({ screen: { width: 100, height: 100, frameId: "f" }, ocr: { blocks: [], text: richText, level: "fast", languages: ["en"], durationMs: 1 } }),
    });
    expect(rich.visionRequired).toBe(false);
  });

  it("requires vision when the instruction refers to visual content", () => {
    const richText = "b".repeat(400);
    const intent = classifyIntent({
      instruction: "What does this chart show?",
      snapshot: makeSnapshot({ screen: { width: 100, height: 100, frameId: "f" }, ocr: { blocks: [], text: richText, level: "fast", languages: ["en"], durationMs: 1 } }),
      mode: makeMode(),
      now: NOW,
    });
    expect(intent.visionRequired).toBe(true);
  });

  it("requires vision in coding mode when OCR confidence is low", () => {
    const intent = classifyIntent({
      instruction: "implement the function shown",
      snapshot: makeSnapshot({
        screen: { width: 100, height: 100, frameId: "f" },
        ocr: {
          blocks: [
            { text: "blurry", confidence: 0.3, boundingBox: { x: 0, y: 0, width: 5, height: 5 } },
            { text: "text", confidence: 0.4, boundingBox: { x: 0, y: 10, width: 5, height: 5 } },
          ],
          text: "c".repeat(300),
          level: "fast",
          languages: ["en"],
          durationMs: 1,
        },
      }),
      mode: makeMode({ id: "coding-interview", responseSchema: "coding" }),
      now: NOW,
    });
    expect(intent.task).toBe("coding");
    expect(intent.visionRequired).toBe(true);
  });
});

describe("mentionsExternalInfo", () => {
  it("matches research cues, urls, entities and future years", () => {
    expect(mentionsExternalInfo("what is the latest on the merger", NOW)).toBe(true);
    expect(mentionsExternalInfo("check https://example.com/pricing", NOW)).toBe(true);
    expect(mentionsExternalInfo("who is Figma", NOW)).toBe(true);
    expect(mentionsExternalInfo("revenue outlook for 2027", NOW)).toBe(true);
    expect(mentionsExternalInfo("how do I reverse a linked list", NOW)).toBe(false);
    expect(mentionsExternalInfo("the 2019 report we shipped", NOW)).toBe(false);
  });
});

describe("classifyIntent answer shapes", () => {
  const ocr = (text: string) => ({
    blocks: [],
    text,
    level: "fast" as const,
    languages: ["en"],
    durationMs: 1,
  });
  const screen = { width: 1600, height: 1000, frameId: "f1" };
  const shapeFor = (instruction: string) =>
    classifyIntent({ instruction, snapshot: makeSnapshot(), mode: makeMode(), now: NOW }).answerShape;

  it("detects a multiple-choice question from its wording", () => {
    expect(shapeFor("Which of the following is true about TCP?")).toBe("choice");
    expect(shapeFor("Select the correct answer: what does DNS resolve?")).toBe("choice");
    expect(shapeFor("Which statement best describes a closure?")).toBe("choice");
  });

  it("detects a multiple-choice question from lettered options on screen (⌘↵, no typed question)", () => {
    const intent = classifyIntent({
      snapshot: makeSnapshot({
        screen,
        ocr: ocr(
          "Question 3 of 20\nWhat does HTTP stand for?\nA. HyperText Transfer Protocol\nB. High Transfer Text Process\nC. Hyperlink Text Transport\nD. None of the above",
        ),
      }),
      mode: makeMode(),
      trigger: "shortcut_capture",
      now: NOW,
    });
    expect(intent.answerShape).toBe("choice");
    expect(intent.task).toBe("answer");
  });

  it("keeps a multiple-choice question about code an answer, not a coding task", () => {
    const intent = classifyIntent({
      snapshot: makeSnapshot({
        screen,
        ocr: ocr(
          [
            "Which of the following calls returns the indices of the two numbers?",
            "A) twoSum(nums, target)",
            "B) sum(nums)",
            "C) indexOf(target)",
            "function twoSum(nums, target) {",
            "  const seen = new Map();",
            "}",
          ].join("\n"),
        ),
      }),
      mode: makeMode(),
      trigger: "shortcut_capture",
      now: NOW,
    });
    expect(intent.answerShape).toBe("choice");
    expect(intent.task).toBe("answer");
    expect(intent.schemaId).toBe("answer");
  });

  it("detects a comparison from two labelled responses on screen", () => {
    const intent = classifyIntent({
      snapshot: makeSnapshot({
        screen,
        ocr: ocr(
          "Response A\nThe capital of Australia is Sydney.\n\nResponse B\nThe capital of Australia is Canberra.\n\nWhich response is better?",
        ),
      }),
      mode: makeMode(),
      trigger: "shortcut_capture",
      now: NOW,
    });
    expect(intent.answerShape).toBe("compare");
    expect(shapeFor("Compare these two answers and tell me which is stronger")).toBe("compare");
    expect(shapeFor("Which one is better for this, Kafka or RabbitMQ?")).toBe("compare");
  });

  it("detects yes/no, fill-in and calculation shapes", () => {
    expect(shapeFor("Is a binary search tree always balanced?")).toBe("boolean");
    expect(shapeFor("True or false: TCP guarantees ordering.")).toBe("boolean");
    expect(shapeFor("Fill in the blank: a ____ join returns only matching rows.")).toBe("fill_in");
    expect(shapeFor("What is the total if 12 servers cost $340 each?")).toBe("calculation");
  });

  it("does not read a how/why question opening with an auxiliary as yes/no", () => {
    expect(shapeFor("Can you explain how a mutex works?")).toBe("explain");
    expect(shapeFor("Why is my query slow?")).toBe("explain");
  });

  it("uses the spoken shape for detected questions and suggestion modes — unless it is a pick", () => {
    const interview = makeMode({ id: "interview", responseSchema: "suggested-response" });
    const spoken = classifyIntent({
      snapshot: makeSnapshot(),
      mode: interview,
      detectedEvent: eventFor("behavioral_question", "Tell me about a time you led a team through a hard deadline."),
      trigger: "detected_event",
      now: NOW,
    });
    expect(spoken.answerShape).toBe("spoken");
    const pick = classifyIntent({
      snapshot: makeSnapshot(),
      mode: interview,
      detectedEvent: eventFor("technical_question", "Which one is better here, Kafka or RabbitMQ?"),
      trigger: "detected_event",
      now: NOW,
    });
    expect(pick.answerShape).toBe("compare");
    const generate = classifyIntent({ snapshot: makeSnapshot(), mode: makeMode(), trigger: "shortcut_generate", now: NOW });
    expect(generate.answerShape).toBe("spoken");
  });

  it("derives code, design and summary shapes from the task", () => {
    const coding = classifyIntent({
      snapshot: makeSnapshot({
        screen,
        ocr: ocr(
          "Example 1:\nInput: nums = [2,7,11,15], target = 9\nOutput: [0,1]\nConstraints:\n2 <= nums.length <= 10^4\nfunction twoSum(nums, target) {",
        ),
      }),
      mode: makeMode(),
      now: NOW,
    });
    expect(coding.answerShape).toBe("code");
    expect(shapeFor("How would you design a rate limiter for our public API?")).toBe("design");
    expect(shapeFor("Summarize what we covered so far")).toBe("summary");
  });

  it("detects the written shape and defaults open questions to explain or a short answer", () => {
    expect(shapeFor("Write a reply to this email declining politely")).toBe("written");
    expect(shapeFor("What port does Postgres listen on?")).toBe("short_answer");
    expect(shapeFor("Explain the difference between a process and a thread")).toBe("explain");
    expect(shapeFor("")).toBe("explain");
  });
});

describe("classifyIntent for questions heard live", () => {
  it("answers a detected interview question as speech, never as research, at the mode's latency", () => {
    const intent = classifyIntent({
      snapshot: makeSnapshot(),
      mode: makeMode({ id: "interview", responseSchema: "suggested-response", preferredLatency: "ultra-fast" }),
      detectedEvent: eventFor("question", "Why do you want to leave your current role?"),
      trigger: "detected_event",
      now: NOW,
    });
    expect(intent.task).toBe("answer");
    expect(intent.latency).toBe("ultra-fast");
    expect(intent.answerShape).toBe("spoken");
    // The same words typed into the HUD still take the research path.
    expect(
      classifyIntent({
        snapshot: makeSnapshot(),
        mode: makeMode(),
        instruction: "What is the latest news about our current competitor?",
        trigger: "typed",
        now: NOW,
      }).task,
    ).toBe("research");
  });
});
