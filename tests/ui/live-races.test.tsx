/**
 * Live-loop races (TEST-003): a live suggestion against Esc/Stop, a manual ask, ⌘⇧↵,
 * Retry/Regenerate, a hidden HUD, Stop listening and a mode switch.
 */

import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useAsk } from "@/features/hud/useAsk";
import { PREPARED_TTL_MS } from "@/lib/engine-contract";
import type { MockTransport } from "@/lib/tauri/mock";
import type { DetectedEvent } from "@/lib/types";
import { useAppStore } from "@/stores/appStore";
import { useChatStore } from "@/stores/chatStore";
import { setEngine } from "@/stores/engine";
import { usePanelStore } from "@/stores/panelStore";
import { useProactiveStore } from "@/stores/proactive";
import { useSettingsStore } from "@/stores/settingsStore";
import { makeResponse, ProactiveFakeEngine, setupMockApp } from "./helpers";

function detected(id: string): DetectedEvent {
  return {
    id,
    type: "question",
    confidence: 0.9,
    requiresResponse: true,
    text: `Question ${id}?`,
    segmentIds: [],
    speaker: "Interviewer",
    detectedAt: new Date().toISOString(),
  };
}

const flush = () => act(() => new Promise<void>((resolve) => setTimeout(resolve, 0)));

function patchStatus(patch: Partial<NonNullable<ReturnType<typeof useAppStore.getState>["status"]>>): void {
  const status = useAppStore.getState().status;
  if (status) useAppStore.getState().setStatus({ ...status, ...patch });
}

function setHudVisible(visible: boolean): void {
  usePanelStore.setState((store) => ({ state: store.state ? { ...store.state, visible } : store.state }));
}

describe("live suggestion races", () => {
  let mock: MockTransport;
  let engine: ProactiveFakeEngine;

  beforeEach(async () => {
    mock = await setupMockApp();
    engine = new ProactiveFakeEngine();
    setEngine(engine);
    await useSettingsStore.getState().update({ ai: { proactivePreparation: true, suggestionDisplay: "live" } });
  });

  it("Stop ends a live suggestion's stream: the turn reads Stopped and nothing completes", async () => {
    engine.hold = true;
    const { result } = renderHook(() => useAsk());
    mock.emit("question.detected", detected("q1"));
    await waitFor(() => expect(useChatStore.getState().phase).toBe("streaming"));

    await act(() => result.current.stop());
    engine.release();
    await flush();

    expect(engine.cancelledPrepares).toHaveLength(1);
    expect(useChatStore.getState().turns.at(-1)?.status).toBe("cancelled");
    expect(useChatStore.getState().turns.at(-1)?.response?.content).toBe("Prepared for"); // the draft, never completed
  });

  it("New chat during a live suggestion ends its stream and leaves an empty thread", async () => {
    engine.hold = true;
    const { result } = renderHook(() => useAsk());
    mock.emit("question.detected", detected("q1"));
    await waitFor(() => expect(useChatStore.getState().phase).toBe("streaming"));

    act(() => result.current.newChat());
    engine.release();
    await flush();

    expect(engine.cancelledPrepares).toHaveLength(1);
    expect(useChatStore.getState().turns).toHaveLength(0);
    expect(useChatStore.getState().prepared).toBeNull();
  });

  it("a corrected question waiting behind a suggestion replaces the one it corrects", async () => {
    engine.hold = true;
    mock.emit("question.detected", detected("q1"));
    await waitFor(() => expect(useChatStore.getState().phase).toBe("streaming"));
    mock.emit("question.detected", { ...detected("q2"), text: "How do you size a thread pool?" });
    mock.emit("question.detected", { ...detected("q3"), text: "Sorry — how do you size a connection pool?" });

    engine.release();
    await waitFor(() => expect(engine.prepared).toHaveLength(2));
    engine.release();
    await waitFor(() => expect(useChatStore.getState().turns.at(-1)?.status).toBe("done"));

    expect(engine.prepared.map((input) => input.detectedEvent?.id)).toEqual(["q1", "q3"]);
    expect(useChatStore.getState().turns.map((turn) => turn.suggestion?.question)).toEqual([
      "Question q1?",
      "Sorry — how do you size a connection pool?",
    ]);
  });

  it("a manual ask cancels the live suggestion and owns the thread", async () => {
    engine.hold = true;
    const { result } = renderHook(() => useAsk());
    mock.emit("question.detected", detected("q1"));
    await waitFor(() => expect(useChatStore.getState().phase).toBe("streaming"));

    act(() => {
      result.current.ask({ trigger: "typed", instruction: "What about caching?" });
    });
    engine.release();
    await flush();

    expect(engine.cancelledPrepares).toHaveLength(1);
    const turns = useChatStore.getState().turns;
    expect(turns.map((turn) => turn.status)).toEqual(["cancelled", "streaming"]);
    expect(turns.at(-1)?.prompt).toBe("What about caching?");
  });

  it("⌘⇧↵ over a streaming answer stops it, shows the prepared one and saves it", async () => {
    const { result } = renderHook(() => useAsk());
    act(() => {
      result.current.ask({ trigger: "typed", instruction: "Slow question" });
    });
    engine.preparedQueue.push(makeResponse({ id: "prep-x", prompt: "Why us?", prepared: true }));

    act(() => result.current.generateOrTakePrepared());
    await flush();

    const turns = useChatStore.getState().turns;
    expect(turns.map((turn) => turn.status)).toEqual(["cancelled", "done"]);
    expect(engine.cancelled).toBe(true);
    expect(engine.committed.map((response) => response.id)).toEqual(["prep-x"]);
    expect(engine.committed[0]?.prepared).toBeUndefined();
  });

  it("a live suggestion continues the thread with the answers already given", async () => {
    const { result } = renderHook(() => useAsk());
    act(() => {
      result.current.ask({ trigger: "typed", instruction: "First" });
    });
    act(() => engine.complete(makeResponse({ id: "answer-1", content: "First answer" })));
    mock.emit("question.detected", detected("q2"));

    await waitFor(() => expect(engine.prepared).toHaveLength(1));
    expect(engine.prepared[0]?.previousResponses?.map((response) => response.id)).toEqual(["answer-1"]);
  });

  it("Retry re-sends the failed turn's own request, not the last turn's", async () => {
    const { result } = renderHook(() => useAsk());
    act(() => {
      result.current.ask({ trigger: "shortcut_capture", captureScreen: true, promptLabel: "Assist" });
    });
    act(() => engine.fail({ kind: "ai", code: "ai.stream_failed", message: "x", recoverable: true }));
    act(() => {
      result.current.ask({ trigger: "typed", instruction: "Later question" });
    });
    act(() => engine.complete(makeResponse({ id: "answer-2" })));
    const failed = useChatStore.getState().turns[0];

    act(() => result.current.retry(failed?.id));

    expect(engine.asks.at(-1)).toMatchObject({ trigger: "shortcut_capture", captureScreen: true });
    expect(engine.asks.at(-1)?.instruction).toBeUndefined();
  });

  it("Regenerate on a suggestion turn keeps its detected question", async () => {
    const { result } = renderHook(() => useAsk());
    mock.emit("question.detected", detected("q1"));
    await waitFor(() => expect(useChatStore.getState().turns.at(-1)?.status).toBe("done"));

    act(() => result.current.regenerate(useChatStore.getState().turns[0]?.id));

    expect(engine.asks.at(-1)).toMatchObject({ trigger: "regenerate", detectedEvent: { id: "q1" } });
    expect(useChatStore.getState().turns.at(-1)?.suggestion?.question).toBe("Question q1?");
  });

  it("a hidden HUD gets no live turn; the newest question is prepared when it is shown", async () => {
    setHudVisible(false);
    mock.emit("question.detected", detected("q1"));
    mock.emit("question.detected", detected("q2"));
    await flush();
    expect(engine.prepared).toHaveLength(0);

    act(() => setHudVisible(true));

    await waitFor(() => expect(engine.prepared).toHaveLength(1));
    expect(engine.prepared[0]?.detectedEvent?.id).toBe("q2");
  });

  it("a question detected while hidden is dropped once it is older than the TTL", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      setHudVisible(false);
      mock.emit("question.detected", detected("q-old"));
      vi.setSystemTime(Date.now() + PREPARED_TTL_MS + 1);
      act(() => setHudVisible(true));
      await flush();
      expect(engine.prepared).toHaveLength(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("Stop listening drops the question still waiting for its turn", async () => {
    patchStatus({ audioActive: true });
    engine.hold = true;
    mock.emit("question.detected", detected("q1"));
    await waitFor(() => expect(engine.prepared).toHaveLength(1));
    mock.emit("question.detected", detected("q2")); // queued behind q1

    act(() => patchStatus({ audioActive: false }));
    engine.release();
    await flush();

    expect(engine.prepared.map((input) => input.detectedEvent?.id)).toEqual(["q1"]);
  });

  it("switching mode drops answers prepared for the previous one", async () => {
    await useSettingsStore.getState().update({ ai: { suggestionDisplay: "on_request" } });
    mock.emit("question.detected", detected("q1"));
    await waitFor(() => expect(useChatStore.getState().prepared).not.toBeNull());

    act(() => patchStatus({ modeId: "coding" }));

    expect(useChatStore.getState().prepared).toBeNull();
    expect(useProactiveStore.getState().preparedEventId).toBeNull();
    expect(engine.preparedQueue).toHaveLength(0);
    expect(engine.clearPreparedCalls).toBe(1);
  });

  it("the ⌘⇧↵ hint expires with the prepared answer", () => {
    vi.useFakeTimers();
    try {
      useChatStore.getState().setPrepared(makeResponse({ id: "prep-old", prepared: true }));
      vi.advanceTimersByTime(PREPARED_TTL_MS - 1);
      expect(useChatStore.getState().prepared?.id).toBe("prep-old");
      vi.advanceTimersByTime(2);
      expect(useChatStore.getState().prepared).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });
});
