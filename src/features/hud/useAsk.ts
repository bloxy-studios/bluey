import { useCallback } from "react";

import type { AskTrigger, EngineHandle } from "@/lib/engine-contract";
import { bluey } from "@/lib/tauri/api";
import type { DetectedEvent } from "@/lib/types";
import { useAppStore } from "@/stores/appStore";
import { completedResponses, useChatStore } from "@/stores/chatStore";
import { getEngine } from "@/stores/engine";
import { modeById, useModesStore } from "@/stores/modesStore";
import { useProactiveStore } from "@/stores/proactive";
import { useSessionStore } from "@/stores/sessionStore";
import { useSettingsStore } from "@/stores/settingsStore";

let currentHandle: EngineHandle | null = null;

export interface AskRequest {
  trigger: AskTrigger;
  instruction?: string;
  captureScreen?: boolean;
  promptLabel?: string;
  detectedEvent?: DetectedEvent;
  /** The global shortcut's keydown on Bluey's monotonic clock (fast-path trace). */
  triggeredAtMs?: number;
}

/**
 * Engine glue: turns UI intents into `engine.ask` calls and mirrors the
 * streaming lifecycle into the chat store (generation-guarded, so a stale
 * stream can never overwrite a newer one).
 */
export function useAsk() {
  const ask = useCallback((request: AskRequest): EngineHandle | null => {
    const settings = useSettingsStore.getState().settings;
    const modes = useModesStore.getState().modes;
    const status = useAppStore.getState().status;
    if (!settings) return null;
    const mode =
      modeById(modes, status?.modeId) ?? modeById(modes, settings.general.defaultModeId) ?? modes[0];
    if (!mode) return null;

    const chat = useChatStore.getState();
    const previous = completedResponses(chat.turns);
    const generation = chat.begin(request.instruction, request.promptLabel);
    const sessionState = useSessionStore.getState();

    const handle = getEngine().ask(
      {
        trigger: request.trigger,
        instruction: request.instruction,
        captureScreen: request.captureScreen ?? false,
        mode,
        session: sessionState.active,
        settings,
        previousResponses: previous.length > 0 ? previous : undefined,
        detectedEvent: request.detectedEvent,
        sessionEvents:
          sessionState.active && sessionState.events.length > 0 ? sessionState.events : undefined,
        triggeredAtMs: typeof request.triggeredAtMs === "number" ? request.triggeredAtMs : undefined,
      },
      {
        onPhase: (phase) => useChatStore.getState().setPhase(generation, phase),
        onDraft: (response) => useChatStore.getState().applyDraft(generation, response),
        onComplete: (response) => useChatStore.getState().complete(generation, response),
        onError: (error) => useChatStore.getState().fail(generation, error),
      },
    );
    currentHandle = handle;
    useChatStore.getState().setActiveRequest(generation, handle.requestId);
    return handle;
  }, []);

  const stop = useCallback(async () => {
    const chat = useChatStore.getState();
    if (chat.phase && ["capturing", "analyzing", "thinking", "streaming"].includes(chat.phase)) {
      chat.markCancelled(chat.generation);
      try {
        await currentHandle?.cancel();
      } catch (error) {
        console.warn("[ask] cancel failed", error);
      }
    }
  }, []);

  /**
   * ⌘⇧↵ — show the response prepared for the question currently surfaced
   * (then any other prepared response); otherwise generate from the transcript.
   */
  const generateOrTakePrepared = useCallback((triggeredAtMs?: number) => {
    const engine = getEngine();
    const preparedEventId = useProactiveStore.getState().preparedEventId;
    const prepared =
      (preparedEventId ? engine.takePrepared(preparedEventId) : null) ??
      engine.takePrepared() ??
      useChatStore.getState().prepared;
    if (prepared) {
      useChatStore.getState().showResponse(prepared, prepared.prompt ?? "Suggestion");
      useChatStore.getState().setPrepared(null);
      useProactiveStore.getState().consumePrepared();
      return;
    }
    ask({
      trigger: "shortcut_generate",
      promptLabel: "Suggested response",
      triggeredAtMs: typeof triggeredAtMs === "number" ? triggeredAtMs : undefined,
    });
  }, [ask]);

  const regenerate = useCallback(() => {
    const turns = useChatStore.getState().turns;
    const last = turns[turns.length - 1];
    ask({
      trigger: "regenerate",
      instruction: last?.prompt,
      promptLabel: last?.promptLabel ?? "Regenerated",
      captureScreen: false,
    });
  }, [ask]);

  const newChat = useCallback(() => {
    void stop();
    useChatStore.getState().newChat();
    void bluey.app.dismissResponse().catch(() => undefined);
  }, [stop]);

  return { ask, stop, generateOrTakePrepared, regenerate, newChat };
}
