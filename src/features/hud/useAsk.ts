import { useCallback } from "react";

import type { EngineHandle, EnginePhase } from "@/lib/engine-contract";
import { bluey } from "@/lib/tauri/api";
import { useAppStore } from "@/stores/appStore";
import {
  completedResponses,
  useChatStore,
  type ChatTurn,
  type SuggestionMeta,
  type TurnRequest,
} from "@/stores/chatStore";
import { getEngine } from "@/stores/engine";
import { useHudUiStore } from "@/stores/hudUiStore";
import { modeById, useModesStore } from "@/stores/modesStore";
import { cancelLiveSuggestion, useProactiveStore } from "@/stores/proactive";
import { useSessionStore } from "@/stores/sessionStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useTranscriptStore } from "@/stores/transcriptStore";

let currentHandle: EngineHandle | null = null;

export interface AskRequest extends TurnRequest {
  /** The global shortcut's keydown on Bluey's monotonic clock (fast-path trace). */
  triggeredAtMs?: number;
  /** Re-asking a suggestion turn keeps it rendered as a suggestion. */
  suggestion?: SuggestionMeta;
}

const BUSY_PHASES: ReadonlySet<EnginePhase> = new Set(["capturing", "analyzing", "thinking", "streaming"]);

/**
 * Stop whatever is streaming into the thread — the user's answer and a live suggestion —
 * so its turn reads "Stopped" and nothing it produced is saved (LIVE-001). `dismissed`
 * when the user stopped it (Stop, Esc): only then does a live suggestion count against
 * the proactive gate; taking the prepared answer or starting a new chat is no rejection.
 */
async function cancelStreaming({ dismissed }: { dismissed: boolean }): Promise<void> {
  const chat = useChatStore.getState();
  if (chat.phase && BUSY_PHASES.has(chat.phase)) chat.markCancelled(chat.generation);
  const handle = currentHandle;
  currentHandle = null;
  await Promise.all([
    handle?.cancel().catch((error: unknown) => console.warn("[ask] cancel failed", error)),
    cancelLiveSuggestion({ dismissed }),
  ]);
}

/**
 * A turn's request; turns shown from the prepared cache fall back to their
 * question. A heard question goes back as the detected question it was, never
 * as a typed instruction: it is speech, not my words (AI-004).
 */
function requestOf(turn: ChatTurn): TurnRequest {
  if (turn.request) return turn.request;
  if (turn.suggestion) {
    const { question, speaker } = turn.suggestion;
    return {
      trigger: "detected_event",
      promptLabel: turn.promptLabel,
      detectedEvent: {
        id: `suggestion:${turn.id}`,
        type: "question",
        confidence: 1,
        requiresResponse: true,
        text: question,
        segmentIds: [],
        ...(speaker ? { speaker } : {}),
        detectedAt: turn.response?.createdAt ?? new Date().toISOString(),
      },
    };
  }
  return {
    trigger: "regenerate",
    instruction: turn.prompt,
    promptLabel: turn.promptLabel,
    captureScreen: false,
  };
}

function findTurn(turnId?: string): ChatTurn | undefined {
  const turns = useChatStore.getState().turns;
  return turnId ? turns.find((turn) => turn.id === turnId) : turns.at(-1);
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

    // A manual ask replaces a live suggestion streaming into the thread (LIVE-001).
    void cancelLiveSuggestion();
    const chat = useChatStore.getState();
    const previous = completedResponses(chat.turns);
    const { triggeredAtMs: _triggeredAtMs, suggestion, ...turnRequest } = request;
    const generation = chat.begin(request.instruction, request.promptLabel, {
      request: turnRequest,
      ...(suggestion ? { suggestion } : {}),
    });
    const sessionState = useSessionStore.getState();

    const handle = getEngine().ask(
      {
        trigger: request.trigger,
        instruction: request.instruction,
        captureScreen: request.captureScreen ?? false,
        screenAllowed: useHudUiStore.getState().screenEnabled,
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
    void handle.done.finally(() => {
      if (currentHandle === handle) currentHandle = null;
    });
    useChatStore.getState().setActiveRequest(generation, handle.requestId);
    return handle;
  }, []);

  const stop = useCallback(() => cancelStreaming({ dismissed: true }), []);

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
      // A turn still streaming is stopped, never left spinning above the answer (LIVE-011).
      void cancelStreaming({ dismissed: false });
      // Shown as a suggestion turn: the question it answers (and who asked, when known).
      const detected = preparedEventId
        ? useTranscriptStore.getState().questions.find((question) => question.id === preparedEventId)
        : undefined;
      const question = prepared.prompt ?? detected?.text;
      useChatStore.getState().showResponse(prepared, {
        promptLabel: question ?? "Suggestion",
        suggestion: question ? { question, ...(detected?.speaker ? { speaker: detected.speaker } : {}) } : undefined,
        ...(detected
          ? { request: { trigger: "detected_event", detectedEvent: detected, promptLabel: detected.text } }
          : {}),
      });
      useChatStore.getState().setPrepared(null);
      useProactiveStore.getState().consumePrepared();
      // On screen now, so it belongs to the session like any answer (DATA-007).
      void engine
        .commitShown(prepared, useSessionStore.getState().active)
        .catch((error: unknown) => console.warn("[ask] saving the prepared answer failed", error));
      return;
    }
    ask({
      trigger: "shortcut_generate",
      promptLabel: "Suggested response",
      triggeredAtMs: typeof triggeredAtMs === "number" ? triggeredAtMs : undefined,
    });
  }, [ask]);

  /** Re-send a (failed) turn's original request: same trigger, screen and question (UX-011). */
  const retry = useCallback(
    (turnId?: string) => {
      const turn = findTurn(turnId);
      if (turn) ask({ ...requestOf(turn), ...(turn.suggestion ? { suggestion: turn.suggestion } : {}) });
    },
    [ask],
  );

  /** Ask a turn's question again for a different answer, keeping its context (UX-011). */
  const regenerate = useCallback(
    (turnId?: string) => {
      const turn = findTurn(turnId);
      if (!turn) return;
      ask({
        ...requestOf(turn),
        trigger: "regenerate",
        ...(turn.suggestion ? { suggestion: turn.suggestion } : {}),
      });
    },
    [ask],
  );

  const newChat = useCallback(() => {
    void cancelStreaming({ dismissed: false });
    useChatStore.getState().newChat();
    void bluey.app.dismissResponse().catch(() => undefined);
  }, []);

  return { ask, stop, generateOrTakePrepared, retry, regenerate, newChat };
}
