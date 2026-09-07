import {
  allocateBudget,
  compressKeepHead,
  compressKeepTail,
  defaultContextBudget,
  TRUNCATION_MARKER,
} from "@/context/budget";
import { estimateTokens } from "@/context/fusion";
import type { ContextItem, ContextSource } from "@/lib/types";
import { makeSettings } from "../../fixtures/helpers/builders";

function item(source: ContextSource, content: string, relevance = 0.5, ref?: string): ContextItem {
  return { source, content, relevance, tokens: estimateTokens(content), ref };
}

const LONG_TRANSCRIPT = Array.from({ length: 60 }, (_, i) => `Speaker: line number ${i} of the conversation`).join("\n");
const LONG_OCR = Array.from({ length: 60 }, (_, i) => `Screen row ${i} with some visible words`).join("\n");

describe("allocateBudget", () => {
  it("respects the token budget", () => {
    const items = [
      item("user_instruction", "What is happening here?", 1),
      item("ocr", LONG_OCR, 0.6),
      item("transcript", LONG_TRANSCRIPT, 0.7),
      item("session_memory", "Earlier response about databases and indexes.", 0.5),
    ];
    const result = allocateBudget(items, 200);
    expect(result.totalTokens).toBeLessThanOrEqual(200);
  });

  it("always keeps the user instruction, even under a tiny budget", () => {
    const items = [
      item("user_instruction", "Explain the stack trace on my screen please", 1),
      item("ocr", LONG_OCR, 0.9),
    ];
    const result = allocateBudget(items, 8);
    expect(result.included.some((i) => i.source === "user_instruction")).toBe(true);
  });

  it("compresses transcript by keeping the tail", () => {
    const items = [
      item("user_instruction", "What did we decide?", 1),
      item("transcript", LONG_TRANSCRIPT, 0.8, "segment:all"),
    ];
    const result = allocateBudget(items, 80);
    const transcript = result.included.find((i) => i.source === "transcript");
    expect(transcript).toBeDefined();
    expect(transcript!.content).toContain(TRUNCATION_MARKER);
    expect(transcript!.content).toContain("line number 59"); // tail survives
    expect(transcript!.content).not.toContain("line number 0 "); // head dropped
    expect(result.compressed).toContain("segment:all");
  });

  it("compresses OCR by keeping the head with a marker", () => {
    const items = [
      item("user_instruction", "What is on screen?", 1),
      item("ocr", LONG_OCR, 0.8, "ocr"),
    ];
    const result = allocateBudget(items, 80);
    const ocr = result.included.find((i) => i.source === "ocr");
    expect(ocr).toBeDefined();
    expect(ocr!.content).toContain("Screen row 0"); // head survives
    expect(ocr!.content).not.toContain("Screen row 59");
    expect(ocr!.content.trimEnd().endsWith(TRUNCATION_MARKER)).toBe(true);
  });

  it("drops lowest-relevance items first within a priority class and notes omissions", () => {
    const keepable = item("document", "High relevance chunk ".repeat(10), 0.9, "chunk:hi");
    const droppable = item("document", "Low relevance chunk ".repeat(10), 0.2, "chunk:lo");
    const budget = estimateTokens(keepable.content) + 4;
    const result = allocateBudget([keepable, droppable], budget);
    expect(result.included.map((i) => i.ref)).toContain("chunk:hi");
    expect(result.dropped.map((i) => i.ref)).toContain("chunk:lo");
    expect(result.omittedNote).toMatch(/document/);
  });

  it("prioritizes recent transcript and OCR over old transcript and session memory", () => {
    const items = [
      item("transcript_old", "old talk ".repeat(30), 0.9, "old"),
      item("transcript", "fresh talk ".repeat(30), 0.4, "fresh"),
      item("session_memory", "memory ".repeat(30), 0.9, "mem"),
      item("ocr", "screen text ".repeat(30), 0.4, "ocr"),
    ];
    const budget = estimateTokens("fresh talk ".repeat(30)) + estimateTokens("screen text ".repeat(30)) + 4;
    const result = allocateBudget(items, budget);
    const refs = result.included.map((i) => i.ref);
    expect(refs).toContain("fresh");
    expect(refs).toContain("ocr");
    expect(refs).not.toContain("old");
  });
});

describe("compress helpers", () => {
  it("keep-tail keeps the last lines under the budget", () => {
    const compressed = compressKeepTail(LONG_TRANSCRIPT, 40);
    expect(estimateTokens(compressed)).toBeLessThanOrEqual(40);
    expect(compressed.startsWith(TRUNCATION_MARKER)).toBe(true);
    expect(compressed).toContain("line number 59");
  });

  it("keep-head keeps the first lines under the budget", () => {
    const compressed = compressKeepHead(LONG_OCR, 40);
    expect(estimateTokens(compressed)).toBeLessThanOrEqual(40);
    expect(compressed).toContain("Screen row 0");
    expect(compressed.trimEnd().endsWith(TRUNCATION_MARKER)).toBe(true);
  });

  it("returns text unchanged when it already fits", () => {
    expect(compressKeepTail("short", 100)).toBe("short");
    expect(compressKeepHead("short", 100)).toBe("short");
  });
});

describe("defaultContextBudget", () => {
  it("subtracts response headroom from the configured budget", () => {
    const settings = makeSettings({ ai: { contextTokenBudget: 8000 } });
    expect(defaultContextBudget(settings)).toBe(8000 - 1024);
    expect(defaultContextBudget(settings, 2000)).toBe(6000);
  });

  it("never goes below a sane floor", () => {
    const settings = makeSettings({ ai: { contextTokenBudget: 600 } });
    expect(defaultContextBudget(settings)).toBe(512);
  });
});
