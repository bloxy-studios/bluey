import { enrichSnapshot, snapshotOptionsFor } from "@/context/snapshot";
import type { RetrievedChunk } from "@/lib/types";
import { makeMode, makeResponse, makeSession, makeSettings, makeSnapshot } from "../../fixtures/helpers/builders";

describe("snapshotOptionsFor", () => {
  const settings = makeSettings({ screen: { ocrLevel: "fast", maxImageDimension: 1400 } });

  it("includes the screen only when the mode requires it or the trigger is a capture", () => {
    const noScreenMode = makeMode({ contextRequirements: ["transcript"] });
    expect(snapshotOptionsFor({ mode: noScreenMode, settings, trigger: "typed" }).includeScreen).toBe(false);
    expect(snapshotOptionsFor({ mode: noScreenMode, settings, trigger: "shortcut_capture" }).includeScreen).toBe(true);

    const screenMode = makeMode({ contextRequirements: ["screen"] });
    expect(snapshotOptionsFor({ mode: screenMode, settings, trigger: "typed" }).includeScreen).toBe(true);
  });

  it("derives OCR level and capture options from settings", () => {
    const options = snapshotOptionsFor({
      mode: makeMode({ contextRequirements: ["screen"] }),
      settings,
      trigger: "shortcut_capture",
    });
    expect(options.includeOcr).toBe(true);
    expect(options.inlineImage).toBe(true);
    expect(options.ocrLevel).toBe("fast");
    expect(options.capture?.maxDimension).toBe(1400);
  });

  it("includes the transcript for transcript modes and generate/detected triggers", () => {
    const transcriptMode = makeMode({ contextRequirements: ["transcript"] });
    const silentMode = makeMode({ contextRequirements: [] });
    expect(snapshotOptionsFor({ mode: transcriptMode, settings, trigger: "typed" }).includeTranscript).toBe(true);
    expect(snapshotOptionsFor({ mode: silentMode, settings, trigger: "typed" }).includeTranscript).toBe(false);
    expect(snapshotOptionsFor({ mode: silentMode, settings, trigger: "shortcut_generate" }).includeTranscript).toBe(true);
    expect(
      snapshotOptionsFor({ mode: transcriptMode, settings, trigger: "typed", transcriptWindowSeconds: 60 })
        .transcriptWindowSeconds,
    ).toBe(60);
  });
});

describe("enrichSnapshot", () => {
  const settings = makeSettings();
  const mode = makeMode({ responseStyle: { length: "concise" } });

  it("fills mode context with the effective style", () => {
    const enriched = enrichSnapshot(makeSnapshot(), { mode, settings });
    expect(enriched.mode?.mode.id).toBe(mode.id);
    expect(enriched.mode?.responseStyle).toEqual({ length: "concise", tone: "natural" });
  });

  it("fills session context from the session and previous responses", () => {
    const previous = [makeResponse({ id: "r1", content: "x".repeat(500) })];
    const enriched = enrichSnapshot(makeSnapshot(), {
      mode,
      settings,
      session: makeSession({ id: "ses_7" }),
      previousResponses: previous,
    });
    expect(enriched.session?.sessionId).toBe("ses_7");
    expect(enriched.session?.recentResponses).toHaveLength(1);
    expect(enriched.session?.recentResponses[0]?.content.length).toBeLessThan(400);
  });

  it("splits personal instructions out of retrieved chunks", () => {
    const retrieved: RetrievedChunk[] = [
      { chunkId: "c1", documentId: "d1", documentTitle: "Resume", documentKind: "resume", content: "resume text", score: 0.8, scope: "global" },
      { chunkId: "c2", documentId: "d2", documentTitle: "Prefs", documentKind: "personal_instructions", content: "Always answer briefly.", score: 0.9, scope: "global" },
    ];
    const enriched = enrichSnapshot(makeSnapshot(), { mode, settings, retrieved });
    expect(enriched.userContext?.personalInstructions).toBe("Always answer briefly.");
    expect(enriched.userContext?.chunks.map((c) => c.chunkId)).toEqual(["c1"]);
  });

  it("sets the trimmed user instruction and does not mutate the input", () => {
    const original = makeSnapshot();
    const enriched = enrichSnapshot(original, { mode, settings, instruction: "  hello  " });
    expect(enriched.userInstruction).toBe("hello");
    expect(original.userInstruction).toBeUndefined();
    expect(original.mode).toBeUndefined();
  });
});
