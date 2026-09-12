import { waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import type { MockTransport } from "@/lib/tauri/mock";
import type { DetectedEvent } from "@/lib/types";
import { useChatStore } from "@/stores/chatStore";
import { setEngine } from "@/stores/engine";
import { canShowLive, useProactiveStore } from "@/stores/proactive";
import { useSettingsStore } from "@/stores/settingsStore";
import { useTranscriptStore } from "@/stores/transcriptStore";
import { makeSettings } from "../fixtures/helpers/builders";
import { FakeEngine, ProactiveFakeEngine, makeSegment, setupMockApp } from "./helpers";

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

  it("streams the answer to a detected question into the thread the moment it is heard", async () => {
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

    // Live display: a suggestion turn opened for the question and received the answer — no hint, no chord.
    await waitFor(() => expect(useChatStore.getState().turns.at(-1)?.status).toBe("done"));
    const turn = useChatStore.getState().turns.at(-1);
    expect(useChatStore.getState().turns).toHaveLength(1);
    expect(turn?.suggestion).toEqual({ question: "How would you design a rate limiter?", speaker: "Interviewer" });
    expect(turn?.prompt).toBe("How would you design a rate limiter?");
    expect(turn?.response?.id).toBe("prep-det-1");
    expect(turn?.response?.prepared).toBeUndefined(); // on screen, so no longer "prepared and waiting"
    expect(useChatStore.getState().prepared).toBeNull();
    expect(useProactiveStore.getState().preparedEventId).toBeNull();
    expect(useProactiveStore.getState().preparingEventId).toBeNull();
    expect(useProactiveStore.getState().liveEventId).toBeNull();
    expect(useTranscriptStore.getState().questions.map((q) => q.id)).not.toContain("det-1"); // answered
  });

  it("keeps the ⌘⇧↵ hint when suggestions are shown on request", async () => {
    await useSettingsStore.getState().update({ ai: { suggestionDisplay: "on_request" } });
    mock.emit("transcript.final", makeSegment({ text: "How would you design a rate limiter?" }));

    // response.prepared → chat store; the surfaced question is remembered for ⌘⇧↵.
    await waitFor(() => expect(useChatStore.getState().prepared?.id).toBe("prep-det-1"));
    expect(useChatStore.getState().turns).toHaveLength(0);
    expect(useProactiveStore.getState().preparedEventId).toBe("det-1");
    expect(useProactiveStore.getState().preparingEventId).toBeNull();
    expect(useTranscriptStore.getState().questions.map((q) => q.id)).toContain("det-1");
  });

  it("falls back to the hint while another answer is streaming", async () => {
    const generation = useChatStore.getState().begin("What is this error?");
    mock.emit("question.detected", detected("det-busy"));

    await waitFor(() => expect(useChatStore.getState().prepared?.id).toBe("prep-det-busy"));
    expect(useProactiveStore.getState().preparedEventId).toBe("det-busy");
    // The typed ask was not interrupted: still the only turn, still streaming.
    expect(useChatStore.getState().turns).toHaveLength(1);
    expect(useChatStore.getState().turns[0]?.status).toBe("streaming");
    expect(useChatStore.getState().generation).toBe(generation);
  });

  it("decides live display from the setting and the thread's phase", () => {
    const live = makeSettings();
    expect(canShowLive(live, null)).toBe(true);
    expect(canShowLive(live, "done")).toBe(true);
    expect(canShowLive(live, "error")).toBe(true);
    for (const phase of ["capturing", "analyzing", "thinking", "streaming"] as const) {
      expect(canShowLive(live, phase)).toBe(false);
    }
    expect(canShowLive(makeSettings({ ai: { suggestionDisplay: "on_request" } }), null)).toBe(false);
    expect(canShowLive(null, null)).toBe(false);
  });

  it("ignores statements and does not prepare for events that need no response", async () => {
    mock.emit("transcript.final", makeSegment({ text: "Let's move on to the next topic." }));
    mock.emit("question.detected", detected("det-note", false));
    await flush();
    expect(engine.classified).toHaveLength(1);
    expect(engine.prepared).toHaveLength(0);
    expect(useChatStore.getState().prepared).toBeNull();
    expect(useChatStore.getState().turns).toHaveLength(0);
  });

  it("does nothing when proactive preparation is off", async () => {
    await useSettingsStore.getState().update({ ai: { proactivePreparation: false } });
    mock.emit("transcript.final", makeSegment({ text: "What is your biggest weakness?" }));
    mock.emit("question.detected", detected("det-off"));
    await flush();
    expect(engine.classified).toHaveLength(0);
    expect(engine.prepared).toHaveLength(0);
    expect(useChatStore.getState().turns).toHaveLength(0);
  });

  it("dedupes question ids and serializes preparation (newest waiting question wins)", async () => {
    await useSettingsStore.getState().update({ ai: { suggestionDisplay: "on_request" } });
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

  it("serializes live suggestions too: each question gets its own turn, in order", async () => {
    engine.hold = true;
    mock.emit("question.detected", detected("live-1"));
    await waitFor(() => expect(useProactiveStore.getState().liveEventId).toBe("live-1"));
    expect(useChatStore.getState().turns.at(-1)?.status).toBe("streaming");
    expect(useChatStore.getState().turns.at(-1)?.response?.content).toBe("Prepared for"); // first draft
    mock.emit("question.detected", detected("live-2"));
    await flush();
    expect(engine.prepared).toHaveLength(1); // waiting behind the streaming one

    engine.hold = false;
    engine.release();
    await waitFor(() => expect(useChatStore.getState().turns).toHaveLength(2));
    await waitFor(() => expect(useChatStore.getState().turns.every((t) => t.status === "done")).toBe(true));
    expect(useChatStore.getState().turns.map((t) => t.suggestion?.question)).toEqual(["Question live-1?", "Question live-2?"]);
    expect(useProactiveStore.getState().liveEventId).toBeNull();
  });

  it("fails a live turn with Regenerate when the engine yields nothing instead of spinning forever", async () => {
    setEngine(new FakeEngine()); // its prepare() resolves null without touching the callbacks
    mock.emit("question.detected", detected("det-silent"));

    await waitFor(() => expect(useChatStore.getState().turns.at(-1)?.status).toBe("error"));
    const turn = useChatStore.getState().turns.at(-1);
    expect(turn?.suggestion?.question).toBe("Question det-silent?");
    expect(turn?.error?.code).toBe("ai.prepare_failed");
    expect(turn?.error?.recovery).toEqual({ type: "retry" });
    expect(useChatStore.getState().phase).toBe("error");
    expect(useProactiveStore.getState().liveEventId).toBeNull();
    expect(useProactiveStore.getState().preparedEventId).toBeNull();
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
    await waitFor(() => expect(useChatStore.getState().turns.at(-1)?.status).toBe("done"));
    expect(useChatStore.getState().turns.at(-1)?.suggestion?.question).toBe("Can you walk me through your résumé?");
    expect(useChatStore.getState().prepared).toBeNull();
  });
});
