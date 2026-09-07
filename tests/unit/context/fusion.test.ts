import { estimateTokens, fuseContext, keywordOverlap, looksLikeQuestion } from "@/context/fusion";
import type { RetrievedChunk } from "@/lib/types";
import { makeSegment, makeSnapshot } from "../../fixtures/helpers/builders";

describe("estimateTokens", () => {
  it("approximates latin text at ~4 chars per token", () => {
    expect(estimateTokens("")).toBe(0);
    expect(estimateTokens("abcd")).toBe(1);
    expect(estimateTokens("abcdefgh")).toBe(2);
    expect(estimateTokens("abcdefghi")).toBe(3);
  });

  it("counts CJK codepoints as ~1 token each", () => {
    expect(estimateTokens("你好")).toBe(2);
    expect(estimateTokens("你好ab")).toBe(3); // 2 CJK + ceil(2/4)
    expect(estimateTokens("こんにちは")).toBe(5);
  });
});

describe("looksLikeQuestion", () => {
  it("detects question marks and rising patterns", () => {
    expect(looksLikeQuestion("Why do you want this role?")).toBe(true);
    expect(looksLikeQuestion("tell me about your last project")).toBe(true);
    expect(looksLikeQuestion("walk me through the design")).toBe(true);
    expect(looksLikeQuestion("We shipped it on Friday.")).toBe(false);
  });
});

describe("fuseContext", () => {
  it("puts the user instruction first with relevance 1.0", () => {
    const snapshot = makeSnapshot({
      transcript: { segments: [makeSegment({ text: "Some chatter." })], windowSeconds: 180 },
    });
    const items = fuseContext(snapshot, { instruction: "Explain this error" });
    expect(items[0]?.source).toBe("user_instruction");
    expect(items[0]?.relevance).toBe(1);
    expect(items[0]?.content).toBe("Explain this error");
  });

  it("decays transcript relevance with age and boosts questions", () => {
    const oldStatement = makeSegment({ id: "s1", text: "We talked about lunch options.", startTime: 0, endTime: 1000 });
    const newStatement = makeSegment({ id: "s2", text: "We talked about lunch options.", startTime: 110_000, endTime: 111_000 });
    const oldQuestion = makeSegment({ id: "s3", text: "What is your biggest weakness?", startTime: 0, endTime: 1000 });
    const snapshot = makeSnapshot({
      transcript: { segments: [oldStatement, newStatement, oldQuestion], windowSeconds: 180 },
    });
    const items = fuseContext(snapshot, { transcriptNowMs: 111_000 });
    const byRef = new Map(items.map((i) => [i.ref, i]));
    const relOldStatement = byRef.get("segment:s1")?.relevance ?? 0;
    const relNewStatement = byRef.get("segment:s2")?.relevance ?? 0;
    const relOldQuestion = byRef.get("segment:s3")?.relevance ?? 0;

    expect(relNewStatement).toBeGreaterThan(relOldStatement);
    expect(relOldQuestion).toBeGreaterThan(relOldStatement);
  });

  it("marks segments older than the recent window as transcript_old", () => {
    const old = makeSegment({ id: "s1", endTime: 1000, startTime: 0 });
    const fresh = makeSegment({ id: "s2", startTime: 200_000, endTime: 201_000 });
    const snapshot = makeSnapshot({ transcript: { segments: [old, fresh], windowSeconds: 180 } });
    const items = fuseContext(snapshot, { transcriptNowMs: 201_000 });
    expect(items.find((i) => i.ref === "segment:s1")?.source).toBe("transcript_old");
    expect(items.find((i) => i.ref === "segment:s2")?.source).toBe("transcript");
  });

  it("scores OCR by keyword overlap with the current question", () => {
    const ocrText = "Given an array of integers nums and an integer target, return indices of the two numbers.";
    const relevant = fuseContext(
      makeSnapshot({ ocr: { blocks: [], text: ocrText, level: "fast", languages: ["en"], durationMs: 5 } }),
      { instruction: "solve the two numbers target indices problem" },
    ).find((i) => i.source === "ocr");
    const irrelevant = fuseContext(
      makeSnapshot({ ocr: { blocks: [], text: ocrText, level: "fast", languages: ["en"], durationMs: 5 } }),
      { instruction: "summarize quarterly marketing revenue" },
    ).find((i) => i.source === "ocr");

    expect(relevant).toBeDefined();
    expect(irrelevant).toBeDefined();
    expect(relevant!.relevance).toBeGreaterThan(irrelevant!.relevance);
  });

  it("boosts OCR containing code markers", () => {
    const plain = fuseContext(
      makeSnapshot({ ocr: { blocks: [], text: "Welcome to the quarterly all hands meeting agenda", level: "fast", languages: ["en"], durationMs: 5 } }),
      {},
    ).find((i) => i.source === "ocr");
    const code = fuseContext(
      makeSnapshot({ ocr: { blocks: [], text: "function add(a, b) { return a + b; }", level: "fast", languages: ["en"], durationMs: 5 } }),
      {},
    ).find((i) => i.source === "ocr");
    expect(code!.relevance).toBeGreaterThan(plain!.relevance);
  });

  it("ranks selected text and focused element high", () => {
    const snapshot = makeSnapshot({
      accessibility: {
        application: { name: "Xcode" },
        elements: [],
        selectedText: "let total = items.reduce(0, +)",
        focusedElement: { role: "AXTextArea", label: "Editor", depth: 2 },
        visibleText: "lots of other text on the screen beyond selection",
        truncated: false,
        capturedAt: "2026-09-07T09:00:00.000Z",
      },
    });
    const items = fuseContext(snapshot, {});
    const selected = items.find((i) => i.ref === "ax:selected");
    const focused = items.find((i) => i.ref === "ax:focused");
    const visible = items.find((i) => i.ref === "ax:visible");
    expect(selected?.relevance).toBe(0.9);
    expect(focused?.relevance).toBe(0.85);
    expect(visible!.relevance).toBeLessThan(0.6);
  });

  it("maps retrieved chunks to resume/job_description sources with their scores", () => {
    const chunks: RetrievedChunk[] = [
      { chunkId: "c1", documentId: "d1", documentTitle: "Resume", documentKind: "resume", content: "Led the payments team.", score: 0.77, scope: "global" },
      { chunkId: "c2", documentId: "d2", documentTitle: "JD", documentKind: "job_description", content: "Own the billing platform.", score: 0.62, scope: "mode" },
      { chunkId: "c3", documentId: "d3", documentTitle: "Notes", documentKind: "notes", content: "misc", score: 0.4, scope: "global" },
    ];
    const items = fuseContext(makeSnapshot({ userContext: { chunks } }), {});
    expect(items.find((i) => i.ref === "chunk:c1")?.source).toBe("resume");
    expect(items.find((i) => i.ref === "chunk:c1")?.relevance).toBe(0.77);
    expect(items.find((i) => i.ref === "chunk:c2")?.source).toBe("job_description");
    expect(items.find((i) => i.ref === "chunk:c3")?.source).toBe("document");
  });

  it("gives personal instructions 0.9 and session memory a moderate score", () => {
    const snapshot = makeSnapshot({
      userContext: { chunks: [], personalInstructions: "Prefer short answers with concrete numbers." },
      session: {
        sessionId: "ses_1",
        modeId: "general",
        startedAt: "2026-09-07T08:00:00.000Z",
        recentResponses: [
          { id: "r1", content: "First earlier answer", createdAt: "2026-09-07T08:10:00.000Z" },
          { id: "r2", content: "Second earlier answer", createdAt: "2026-09-07T08:20:00.000Z" },
        ],
        recentEvents: [],
        notes: [],
        documentIds: [],
      },
    });
    const items = fuseContext(snapshot, {});
    expect(items.find((i) => i.source === "personal_instructions")?.relevance).toBe(0.9);
    const memories = items.filter((i) => i.source === "session_memory");
    expect(memories).toHaveLength(2);
    for (const memory of memories) {
      expect(memory.relevance).toBeGreaterThanOrEqual(0.4);
      expect(memory.relevance).toBeLessThanOrEqual(0.7);
    }
    // Most recent response scores higher.
    const r1 = items.find((i) => i.ref === "response:r1")!.relevance;
    const r2 = items.find((i) => i.ref === "response:r2")!.relevance;
    expect(r2).toBeGreaterThan(r1);
  });
});

describe("keywordOverlap", () => {
  it("is 0 with no shared keywords and grows with overlap", () => {
    expect(keywordOverlap("alpha beta", "gamma delta")).toBe(0);
    expect(keywordOverlap("alpha beta", "alpha delta")).toBeCloseTo(0.5);
  });
});
