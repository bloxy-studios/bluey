/**
 * Screen-side context items: the app/window identity line (CTX-012), the
 * de-duplicated window text (PERF-005), the relevance floor for unrelated
 * typed asks (PERF-006) and the session's notes.
 */

import { PromptBuilder } from "@/ai/prompt-builder";
import { SECTION_LABELS } from "@/ai/prompts/labels";
import { estimateTokens, fuseContext, novelWindowText } from "@/context/fusion";
import type { AccessibilityContext, ContextItem, ContextSnapshot, OCRContext } from "@/lib/types";
import { makeMode, makeSegment, makeSnapshot } from "../../fixtures/helpers/builders";

const OCR_TEXT = [
  "1. Two Sum",
  "Given an array of integers nums and an integer target, return indices of the",
  "two numbers such that they add up to target.",
  "Example 1: Input: nums = [2,7,11,15], target = 9 Output: [0,1]",
].join("\n");

function ocr(text = OCR_TEXT): OCRContext {
  return { blocks: [], text, level: "fast", languages: ["en"], durationMs: 40 };
}

function ax(overrides: Partial<AccessibilityContext> = {}): AccessibilityContext {
  return {
    elements: [],
    truncated: false,
    capturedAt: "2026-09-28T09:00:00.000Z",
    application: { name: "Google Chrome", bundleId: "com.google.Chrome" },
    window: { title: "Two Sum - LeetCode", adapter: "chrome" },
    focusedElement: {
      role: "AXTextArea",
      depth: 3,
      label: "Code editor",
      value: "def twoSum(self, nums, target):\n    pass",
    },
    // The AX tree wraps the statement differently from the OCR lines.
    visibleText: [
      "Two Sum - LeetCode",
      "Given an array of integers nums and an integer target,",
      "return indices of the two numbers such that they add up to target.",
      "def twoSum(self, nums, target):",
      "Accepted 14.2M Submissions 27.1M",
    ].join("\n"),
    ...overrides,
  };
}

function chromeSnapshot(overrides: Partial<ContextSnapshot> = {}): ContextSnapshot {
  return makeSnapshot({
    activeApplication: { name: "Google Chrome", bundleId: "com.google.Chrome" },
    activeWindow: { title: "Two Sum - LeetCode", adapter: "chrome" },
    ocr: ocr(),
    accessibility: ax(),
    ...overrides,
  });
}

function render(items: ContextItem[]): string {
  return new PromptBuilder({
    mode: makeMode(),
    style: { length: "concise", tone: "natural" },
    schemaId: "answer",
    trigger: "shortcut_generate",
    items,
    nonce: "n0",
  }).renderContext();
}

describe("app and window identity (CTX-012)", () => {
  it("adds one compact identity line when screen context was captured", () => {
    const items = fuseContext(chromeSnapshot(), { instruction: "solve this" });
    const identity = items.filter((item) => item.source === "active_app");
    expect(identity).toHaveLength(1);
    expect(identity[0]!.content).toBe("App: Google Chrome (chrome) — Window: Two Sum - LeetCode");
    expect(identity[0]!.tokens).toBeLessThanOrEqual(20);
    expect(render(items)).toContain(
      `<context source="${SECTION_LABELS.active_app}" id="n0">\nApp: Google Chrome`,
    );
  });

  it("leaves the identity out when no screen context was captured (screen off, text-only mode)", () => {
    const snapshot = makeSnapshot({
      activeApplication: { name: "Mail" },
      activeWindow: { title: "Re: offer letter — confidential" },
    });
    expect(
      fuseContext(snapshot, { instruction: "tell me about yourself" }).some((i) => i.source === "active_app"),
    ).toBe(false);
  });
});

describe("window text de-duplication (PERF-005)", () => {
  it("drops the window text when it only repeats the OCR, the focused value and the title", () => {
    const items = fuseContext(chromeSnapshot(), { instruction: "solve this" });
    expect(items.some((item) => item.source === "window_text")).toBe(false);
    // The focused editor stays under "Focused UI".
    expect(items.find((item) => item.ref === "ax:focused")?.source).toBe("accessibility");
  });

  it("keeps only the new lines, labelled as window text rather than focused UI", () => {
    const visibleText = [
      "Given an array of integers nums and an integer target,",
      "Constraints: 2 <= nums.length <= 10^4",
      "Only one valid answer exists.",
      "Follow-up: Can you come up with an algorithm that is less than O(n^2) time complexity?",
    ].join("\n");
    const items = fuseContext(chromeSnapshot({ accessibility: ax({ visibleText }) }), {
      instruction: "solve this",
    });
    const windowText = items.find((item) => item.source === "window_text");
    expect(windowText?.content).toBe(visibleText.split("\n").slice(1).join("\n"));
    expect(render(items)).toContain(`<context source="${SECTION_LABELS.window_text}" id="n0">\nConstraints:`);
  });

  it("matches across different line wrapping and case", () => {
    const seen = ["GIVEN an array of integers\nnums and an integer target"];
    expect(novelWindowText("given an array of integers nums\nand an integer target", seen)).toBeUndefined();
  });
});

describe("relevance floor for unrelated typed asks (PERF-006)", () => {
  const longOcr = Array.from(
    { length: 200 },
    (_, i) => `Row ${i}: quarterly revenue ledger entry ${i * 7}`,
  ).join("\n");
  const segments = Array.from({ length: 6 }, (_, i) =>
    makeSegment({
      id: `s${i}`,
      text: `Turn ${i} about the roadmap.`,
      startTime: i * 5_000,
      endTime: i * 5_000 + 4_000,
    }),
  );
  const snapshot = chromeSnapshot({
    ocr: ocr(longOcr),
    accessibility: ax({ selectedText: "ledger entry 42", visibleText: "Sidebar: Inbox\nDrafts" }),
    transcript: { segments, windowSeconds: 180, earlierSummary: "Kick-off and introductions." },
  });

  it("keeps only the focused/selected UI and the last two turns when the ask shares nothing with the screen", () => {
    const items = fuseContext(snapshot, { instruction: "What's the capital of Australia?" });
    const sources = new Set(items.map((item) => item.source));
    expect(sources.has("ocr")).toBe(false);
    expect(sources.has("window_text")).toBe(false);
    expect(sources.has("transcript_old")).toBe(false);
    expect(items.filter((item) => item.source === "transcript").map((item) => item.ref)).toEqual(
      expect.arrayContaining(["segment:s4", "segment:s5"]),
    );
    expect(items.filter((item) => item.source === "transcript")).toHaveLength(2);
    expect(items.some((item) => item.ref === "ax:selected")).toBe(true);
    const total = items.reduce((sum, item) => sum + item.tokens, 0);
    expect(total).toBeLessThan(estimateTokens(longOcr) / 10);
  });

  it("keeps the screen when the ask points at it", () => {
    const items = fuseContext(snapshot, { instruction: "What's on my screen right now?" });
    expect(items.some((item) => item.source === "ocr")).toBe(true);
  });

  it("keeps the screen when the ask shares a keyword with it", () => {
    const items = fuseContext(snapshot, { instruction: "Total the quarterly revenue" });
    expect(items.some((item) => item.source === "ocr")).toBe(true);
    expect(items.filter((item) => item.source === "transcript")).toHaveLength(6);
  });

  it("never applies to a heard question", () => {
    const items = fuseContext(snapshot, {
      detectedEvent: {
        id: "e1",
        type: "question",
        confidence: 0.9,
        requiresResponse: true,
        text: "What's the capital of Australia?",
        segmentIds: [],
        speaker: "Interviewer",
        detectedAt: "2026-09-28T09:00:00.000Z",
      },
    });
    expect(items.some((item) => item.source === "ocr")).toBe(true);
  });
});

describe("session notes", () => {
  it("renders the session's notes as one session-memory item", () => {
    const items = fuseContext(
      makeSnapshot({
        session: {
          sessionId: "s1",
          modeId: "m1",
          startedAt: "2026-09-28T09:00:00.000Z",
          recentResponses: [],
          recentEvents: [],
          notes: ["Ask about on-call", "Salary band 180-200k"],
          documentIds: [],
        },
      }),
      { instruction: "What should I ask next?" },
    );
    const notes = items.filter((item) => item.ref === "session:notes");
    expect(notes).toHaveLength(1);
    expect(notes[0]!.source).toBe("session_memory");
    expect(notes[0]!.content).toBe(
      "Your notes for this session:\n- Ask about on-call\n- Salary band 180-200k",
    );
  });
});
