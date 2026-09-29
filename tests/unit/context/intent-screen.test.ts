/**
 * classifyIntent reading the screen: the union of OCR and accessibility text
 * (CTX-011), the vision gate (CTX-009) and ⌘↵ keeping the screen as the
 * subject when a question was heard earlier (CTX-001).
 */

import { classifyIntent } from "@/context/relevance";
import type { AccessibilityContext, ContextSnapshot, OCRContext } from "@/lib/types";
import { makeMode, makeSnapshot } from "../../fixtures/helpers/builders";

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
