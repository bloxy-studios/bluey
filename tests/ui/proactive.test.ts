import { waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import type { MockTransport } from "@/lib/tauri/mock";
import type { DetectedEvent } from "@/lib/types";
import { useChatStore } from "@/stores/chatStore";
import { setEngine } from "@/stores/engine";
import { useProactiveStore } from "@/stores/proactive";
import { useSettingsStore } from "@/stores/settingsStore";
import { useTranscriptStore } from "@/stores/transcriptStore";
import { ProactiveFakeEngine, makeSegment, setupMockApp } from "./helpers";

function detected(id: string, requiresResponse = true): DetectedEvent {
  return {
    id,
    type: "question",
    confidence: 0.8,
    requiresResponse,
    text: `Question ${id}?`,
    segmentIds: [],
    detectedAt: new Date().toISOString(),
  };
}

const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

describe("proactive preparation loop", () => {
  let mock: MockTransport;
  let engine: ProactiveFakeEngine;

  beforeEach(async () => {
    mock = await setupMockApp();
    engine = new ProactiveFakeEngine();
    setEngine(engine);
    await useSettingsStore.getState().update({ ai: { proactivePreparation: true } });
  });

  it("classifies finalized transcript and prepares a response for a detected question", async () => {
    mock.emit("transcript.final", makeSegment({ text: "How would you design a rate limiter?" }));

    await waitFor(() => expect(engine.prepared).toHaveLength(1));
    const input = engine.prepared[0];
    expect(input?.trigger).toBe("detected_event");
    expect(input?.captureScreen).toBe(false);
    expect(input?.detectedEvent?.id).toBe("det-1");
    expect(input?.mode.id).toBe("general");

    // The classifier saw the segment with the rolling window (no self-inclusion).
    expect(engine.classified).toHaveLength(1);
    expect(engine.classified[0]?.recent.map((s) => s.id)).not.toContain(engine.classified[0]?.segment.id);

    // response.prepared → chat store; the surfaced question is remembered for ⌘⇧↵.
    await waitFor(() => expect(useChatStore.getState().prepared?.id).toBe("prep-det-1"));
    expect(useProactiveStore.getState().preparedEventId).toBe("det-1");
    expect(useProactiveStore.getState().preparingEventId).toBeNull();
    expect(useTranscriptStore.getState().questions.map((q) => q.id)).toContain("det-1");
  });

  it("ignores statements and does not prepare for events that need no response", async () => {
    mock.emit("transcript.final", makeSegment({ text: "Let's move on to the next topic." }));
    mock.emit("question.detected", detected("det-note", false));
    await flush();
    expect(engine.classified).toHaveLength(1);
    expect(engine.prepared).toHaveLength(0);
    expect(useChatStore.getState().prepared).toBeNull();
  });

  it("does nothing when proactive preparation is off", async () => {
    await useSettingsStore.getState().update({ ai: { proactivePreparation: false } });
    mock.emit("transcript.final", makeSegment({ text: "What is your biggest weakness?" }));
    mock.emit("question.detected", detected("det-off"));
    await flush();
    expect(engine.classified).toHaveLength(0);
    expect(engine.prepared).toHaveLength(0);
  });

  it("dedupes question ids and serializes preparation (newest waiting question wins)", async () => {
    engine.hold = true;
    mock.emit("question.detected", detected("q1"));
    await waitFor(() => expect(engine.prepared).toHaveLength(1));
    expect(useProactiveStore.getState().preparingEventId).toBe("q1");

    mock.emit("question.detected", detected("q1")); // duplicate
    mock.emit("question.detected", detected("q2")); // queued…
    mock.emit("question.detected", detected("q3")); // …and replaced by the newer question
    await flush();
    expect(engine.prepared).toHaveLength(1);

    engine.hold = false;
    engine.release();
    await waitFor(() => expect(engine.prepared).toHaveLength(2));
    expect(engine.prepared.map((p) => p.detectedEvent?.id)).toEqual(["q1", "q3"]);
    await waitFor(() => expect(useProactiveStore.getState().preparedEventId).toBe("q3"));
    expect(useProactiveStore.getState().preparingEventId).toBeNull();
  });

  it("dev simulations flow through the same path", async () => {
    await mock.invoke("dev_simulate", {
      simulation: { type: "question", text: "Can you walk me through your résumé?", speaker: "Interviewer" },
    });
    // The simulation emits transcript.final (classified → det-1) and its own question.detected.
    await waitFor(() => expect(engine.prepared.length).toBeGreaterThanOrEqual(1));
    await waitFor(() => expect(useProactiveStore.getState().preparingEventId).toBeNull());
    const ids = engine.prepared.map((p) => p.detectedEvent?.id);
    expect(new Set(ids).size).toBe(ids.length); // never prepared twice for one id
    expect(useChatStore.getState().prepared).not.toBeNull();
  });
});
