/**
 * classifyIntent reading the screen: the union of OCR and accessibility text
 * (CTX-011), the vision gate (CTX-009) and ⌘↵ keeping the screen as the
 * subject when a question was heard earlier (CTX-001).
 */

import { classifyIntent } from "@/context/relevance";
import type { AccessibilityContext, ContextSnapshot, OCRContext } from "@/lib/types";
import { makeMode, makeSegment, makeSnapshot } from "../../fixtures/helpers/builders";

const NOW = () => new Date("2026-09-07T09:05:00.000Z");
const SCREEN = { width: 1600, height: 1000, frameId: "f1" };

function ocr(text: string): OCRContext {
  return { blocks: [], text, level: "fast", languages: ["en"], durationMs: 1 };
}

function ax(visibleText: string, focusedValue?: string): AccessibilityContext {
  return {
    application: { name: "Google Chrome", bundleId: "com.google.Chrome", pid: 1 },
    elements: [],
    visibleText,
    truncated: false,
    capturedAt: NOW().toISOString(),
    ...(focusedValue ? { focusedElement: { role: "AXTextArea", value: focusedValue, depth: 1 } } : {}),
  };
}

function screenSnapshot(overrides: Partial<ContextSnapshot>): ContextSnapshot {
  return makeSnapshot({ screen: SCREEN, ...overrides });
}

const MCQ = [
  "Question 3 of 20",
  "What does HTTP stand for?",
  "A. HyperText Transfer Protocol",
  "B. High Transfer Text Process",
  "C. Hyperlink Text Transport",
].join("\n");

const PROBLEM = [
  "Given an array of integers nums and an integer target, return indices of the two numbers that add up to target.",
  "Example 1:",
  "Input: nums = [2,7,11,15], target = 9",
  "Output: [0,1]",
  "Constraints:",
  "2 <= nums.length <= 10^4",
].join("\n");

describe("classifyIntent reads the whole screen, not only OCR (CTX-011)", () => {
  it("detects a multiple choice that only the accessibility text carries", () => {
    const intent = classifyIntent({
      snapshot: screenSnapshot({ ocr: ocr(""), accessibility: ax(MCQ) }),
      mode: makeMode(),
      trigger: "shortcut_capture",
      now: NOW,
    });
    expect(intent.answerShape).toBe("choice");
  });

  it("routes a problem statement in the focused editor to coding", () => {
    const intent = classifyIntent({
      snapshot: screenSnapshot({ ocr: ocr(""), accessibility: ax("", PROBLEM) }),
      mode: makeMode(),
      trigger: "shortcut_capture",
      now: NOW,
    });
    expect(intent.task).toBe("coding");
    expect(intent.answerShape).toBe("code");
  });
});

const CHART_OCR = [
  "Quarterly revenue by region",
  "Legend: North America  Europe  APAC",
  "Q1 Q2 Q3 Q4",
  "0",
  "25",
  "50",
  "75",
  "100",
  "Revenue ($M)",
  "Source: internal finance dashboard, FY2026 figures, updated weekly by the analytics team.",
].join("\n");
const BROWSER_CHROME = "Back Forward Reload Address and search bar Bookmarks Tab search Extensions Profile ".repeat(3);

describe("the vision gate (CTX-009)", () => {
  it("attaches the screenshot for a chart on ⌘↵ even when AX chrome and labels add text", () => {
    const intent = classifyIntent({
      snapshot: screenSnapshot({ ocr: ocr(`${CHART_OCR}\n${"Revenue grew in every region. ".repeat(4)}`), accessibility: ax(BROWSER_CHROME) }),
      mode: makeMode(),
      trigger: "shortcut_capture",
      now: NOW,
    });
    expect(intent.visionRequired).toBe(true);
  });

  it("measures text sufficiency on OCR only — accessibility chrome does not make a sparse screen textual", () => {
    const intent = classifyIntent({
      snapshot: screenSnapshot({ ocr: ocr("Figure"), accessibility: ax(BROWSER_CHROME) }),
      mode: makeMode(),
      trigger: "shortcut_capture",
      now: NOW,
    });
    expect(intent.visionRequired).toBe(true);
  });

  it("keeps a dense article text-only", () => {
    const article = "The committee met on Tuesday to review the proposal and agreed on the timeline. ".repeat(8);
    const intent = classifyIntent({
      snapshot: screenSnapshot({ ocr: ocr(article), accessibility: ax(BROWSER_CHROME) }),
      mode: makeMode(),
      trigger: "shortcut_capture",
      now: NOW,
    });
    expect(intent.visionRequired).toBe(false);
  });
});

describe("⌘↵ keeps the screen as the subject (CTX-001)", () => {
  const heard = {
    segments: [
      makeSegment({ id: "s1", text: "I'll share a problem with you now.", startTime: 0, endTime: 2 }),
      makeSegment({ id: "s2", text: "Can you see my screen?", startTime: 3, endTime: 5 }),
    ],
    windowSeconds: 180,
  };
  const modes = [
    makeMode(),
    makeMode({ id: "interview", responseSchema: "suggested-response" }),
    makeMode({ id: "coding-interview", responseSchema: "coding" }),
  ];

  for (const mode of modes) {
    it(`solves the problem on screen, not the question heard earlier (${mode.id})`, () => {
      const intent = classifyIntent({
        snapshot: screenSnapshot({ ocr: ocr(PROBLEM), transcript: heard }),
        mode,
        trigger: "shortcut_capture",
        now: NOW,
      });
      expect(intent.task).toBe("coding");
      expect(intent.schemaId).toBe("coding");
      expect(intent.answerShape).toBe("code");
    });
  }

  it("still answers the heard question on ⌘⇧↵ (what do I say next)", () => {
    const intent = classifyIntent({
      snapshot: screenSnapshot({ ocr: ocr("Inbox (3)"), transcript: heard }),
      mode: makeMode({ id: "interview", responseSchema: "suggested-response" }),
      trigger: "shortcut_generate",
      now: NOW,
    });
    expect(intent.answerShape).toBe("spoken");
  });
});
