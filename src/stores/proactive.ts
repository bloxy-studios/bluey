/**
 * Proactive preparation loop (HUD window only).
 *
 *   transcript.final ──► engine.classify() ──► question.detected ──► engine.prepare()
 *                                                                        │
 *                 live: a suggestion turn in the thread, streamed ◄───────┤
 *                 on request: chatStore.prepared ◄── response.prepared ───┘
 *
 * `classify` runs the transcript heuristics (plus the optional fast model) on
 * every finalized segment and emits `question.detected` when the event needs a
 * response. Every `question.detected` — from the engine, from Rust's dev
 * simulations, or from a future backend classifier — goes through the same
 * `prepare()` path, deduped by event id and serialized (one pipeline run at a
 * time; a newer question replaces a waiting one).
 *
 * How the answer surfaces is `ai.suggestionDisplay`:
 * - **live** (default): the moment the question is detected the HUD opens a
 *   suggestion turn (who asked, the question) and the answer streams into it —
 *   no chord to press. If another answer is streaming right then, the
 *   preparation stays silent and falls back to the hint below.
 * - **on request**: the engine prepares silently, emits `response.prepared`,
 *   the chat store mirrors it and the HUD shows "Bluey has a suggestion · ⌘⇧↵";
 *   `preparedEventId` lets ⌘⇧↵ take the response for the question being shown.
 *
 * Everything is gated on `ai.proactivePreparation`.
 */

import { create } from "zustand";

import type { EngineCallbacks, EnginePhase } from "@/lib/engine-contract";
import { eventBus } from "@/lib/tauri/event-bus";
import { getTransport, type Unlisten } from "@/lib/tauri/transport";
import type { BlueyError, DetectedEvent, Settings, TranscriptSegment } from "@/lib/types";
import { recentSegments } from "@/transcript/window";
import { useAppStore } from "./appStore";
import { shownResponse, useChatStore, type SuggestionMeta } from "./chatStore";
import { getEngine } from "./engine";
import { modeById, useModesStore } from "./modesStore";
import { useSessionStore } from "./sessionStore";
import { useSettingsStore } from "./settingsStore";
import { useTranscriptStore } from "./transcriptStore";

/** Transcript context handed to the classifier alongside the new segment. */
export const CLASSIFY_WINDOW_SECONDS = 45;
/** Event ids remembered for dedupe. */
const MAX_TRACKED_EVENT_IDS = 64;

const BUSY_PHASES: ReadonlySet<EnginePhase> = new Set(["capturing", "analyzing", "thinking", "streaming"]);

/** A live preparation ended with no answer and no verdict — the turn must not spin forever. */
const PREPARE_FAILED: BlueyError = {
  kind: "ai",
  code: "ai.prepare_failed",
  message: "The proactive preparation produced no response.",
  recoverable: true,
  recovery: { type: "retry" },
};

interface ProactiveStore {
  /** Detected event whose response was prepared most recently (what ⌘⇧↵ takes). */
  preparedEventId: string | null;
  /** Detected event currently being prepared, if any. */
  preparingEventId: string | null;
  /** Detected event whose suggestion is streaming into the thread right now. */
  liveEventId: string | null;
  setPreparing(eventId: string | null): void;
  setPrepared(eventId: string | null): void;
  setLive(eventId: string | null): void;
  /** Called when the prepared response was shown (or discarded). */
  consumePrepared(): void;
}

export const useProactiveStore = create<ProactiveStore>((set, get) => ({
  preparedEventId: null,
  preparingEventId: null,
  liveEventId: null,
  setPreparing: (eventId) => set({ preparingEventId: eventId }),
  setPrepared: (eventId) => set({ preparedEventId: eventId }),
  setLive: (eventId) => set({ liveEventId: eventId }),
  consumePrepared: () => {
    const id = get().preparedEventId;
    if (id) useTranscriptStore.getState().consumeQuestion(id);
    set({ preparedEventId: null });
  },
}));

/**
 * Pure: a suggestion may stream into the thread when the user asked for live display
 * and nothing else is streaming there — a live suggestion never interrupts an answer.
 */
export function canShowLive(settings: Settings | null, phase: EnginePhase | null): boolean {
  return settings?.ai.suggestionDisplay === "live" && (phase === null || !BUSY_PHASES.has(phase));
}

function isHudWindow(): boolean {
  try {
    return getTransport().currentWindowLabel() === "main";
  } catch {
    return false;
  }
}

function proactiveEnabled(): boolean {
  return useSettingsStore.getState().settings?.ai.proactivePreparation ?? false;
}

function activeMode() {
  const modes = useModesStore.getState().modes;
  const settings = useSettingsStore.getState().settings;
  const status = useAppStore.getState().status;
  return modeById(modes, status?.modeId) ?? modeById(modes, settings?.general.defaultModeId) ?? modes[0];
}

/** Callbacks that stream a live suggestion into the turn opened for it (generation-guarded). */
function liveCallbacks(generation: number, event: DetectedEvent): EngineCallbacks {
  return {
    onPhase: (phase) => {
      if (phase === "cancelled") useChatStore.getState().markCancelled(generation);
      else useChatStore.getState().setPhase(generation, phase);
    },
    onDraft: (response) => useChatStore.getState().applyDraft(generation, response),
    onComplete: (response) => {
      useChatStore.getState().complete(generation, shownResponse(response));
      useTranscriptStore.getState().consumeQuestion(event.id);
    },
    onError: (error) => useChatStore.getState().fail(generation, error),
  };
}

/**
 * Start the loop. Returns the unsubscribe function (wired from `initStores`).
 * Safe to call in any window: it only acts in the HUD window so settings /
 * onboarding never run a second copy of the pipeline.
 */
export function startProactiveLoop(): Unlisten {
  const seen = new Set<string>();
  let queued: DetectedEvent | null = null;
  let busy = false;

  const remember = (id: string) => {
    seen.add(id);
    if (seen.size > MAX_TRACKED_EVENT_IDS) {
      const oldest = seen.values().next().value;
      if (oldest !== undefined) seen.delete(oldest);
    }
  };

  const prepareFor = async (event: DetectedEvent): Promise<void> => {
    busy = true;
    useProactiveStore.getState().setPreparing(event.id);
    try {
      const settings = useSettingsStore.getState().settings;
      const mode = activeMode();
      if (!settings || !mode) return;
      const sessionState = useSessionStore.getState();
      const chat = useChatStore.getState();
      const live = canShowLive(settings, chat.phase);
      let generation: number | null = null;
      let callbacks: EngineCallbacks | undefined;
      if (live) {
        const suggestion: SuggestionMeta = { question: event.text, ...(event.speaker ? { speaker: event.speaker } : {}) };
        generation = chat.begin(event.text, event.text, { phase: "thinking", suggestion });
        useProactiveStore.getState().setLive(event.id);
        callbacks = liveCallbacks(generation, event);
      }
      const response = await getEngine().prepare(
        {
          trigger: "detected_event",
          captureScreen: false,
          detectedEvent: event,
          mode,
          session: sessionState.active,
          settings,
          sessionEvents:
            sessionState.active && sessionState.events.length > 0 ? sessionState.events : undefined,
        },
        callbacks,
      );
      if (response && !live) useProactiveStore.getState().setPrepared(event.id);
      if (generation !== null && response === null) {
        // Nothing came back and no callback settled the turn (e.g. an unavailable engine).
        const current = useChatStore.getState();
        if (current.generation === generation && current.turns.at(-1)?.status === "streaming") {
          current.fail(generation, PREPARE_FAILED);
        }
      }
    } catch (error) {
      console.warn("[proactive] prepare failed", error);
    } finally {
      busy = false;
      useProactiveStore.getState().setPreparing(null);
      useProactiveStore.getState().setLive(null);
      const next = queued;
      queued = null;
      if (next) void prepareFor(next);
    }
  };

  const onDetected = (event: DetectedEvent): void => {
    if (!isHudWindow() || !proactiveEnabled() || !event.requiresResponse) return;
    if (seen.has(event.id)) return;
    remember(event.id);
    if (busy) {
      queued = event; // newest question wins; a stale one is not worth preparing
      return;
    }
    void prepareFor(event);
  };

  const onFinal = async (segment: TranscriptSegment): Promise<void> => {
    if (!isHudWindow() || !proactiveEnabled()) return;
    const settings = useSettingsStore.getState().settings;
    const mode = activeMode();
    if (!settings || !mode) return;
    const others = useTranscriptStore.getState().segments.filter((s) => s.id !== segment.id);
    try {
      // The engine emits `question.detected` itself when the event needs a response.
      await getEngine().classify({
        segment,
        recent: recentSegments(others, CLASSIFY_WINDOW_SECONDS),
        mode,
        settings,
      });
    } catch (error) {
      console.warn("[proactive] classify failed", error);
    }
  };

  const offFinal = eventBus.on("transcript.final", (segment) => void onFinal(segment));
  const offDetected = eventBus.on("question.detected", onDetected);
  return () => {
    offFinal();
    offDetected();
    queued = null;
  };
}

/** Test helper. */
export function resetProactiveForTest(): void {
  useProactiveStore.setState({ preparedEventId: null, preparingEventId: null, liveEventId: null });
}
