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
