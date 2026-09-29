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

import {
  PREPARED_TTL_MS,
  type CancelHandle,
  type EngineCallbacks,
  type EnginePhase,
} from "@/lib/engine-contract";
import { bluey } from "@/lib/tauri/api";
import { eventBus } from "@/lib/tauri/event-bus";
import { getTransport, type Unlisten } from "@/lib/tauri/transport";
import type { BlueyError, BlueyMode, DetectedEvent, Settings, TranscriptSegment } from "@/lib/types";
import {
  CLASSIFIER_MIN_CONFIDENCE,
  conversationalMode,
  DIRECT_QUESTION_MIN_CONFIDENCE,
  DIRECT_QUESTION_TYPES,
  isOpenFragment,
} from "@/transcript/classifier";
import { notableEntry } from "@/transcript/notable";
import { labelSpeaker } from "@/transcript/speaker";
import { recentSegments } from "@/transcript/window";
import { useAppStore } from "./appStore";
import { completedResponses, shownResponse, useChatStore, type SuggestionMeta } from "./chatStore";
import { getEngine } from "./engine";
import { modeById, useModesStore } from "./modesStore";
import { usePanelStore } from "./panelStore";
import { useSessionStore } from "./sessionStore";
import { useSettingsStore } from "./settingsStore";
import { useTranscriptStore } from "./transcriptStore";

/** Transcript context handed to the classifier alongside the new segment. */
export const CLASSIFY_WINDOW_SECONDS = 45;
/** Event ids remembered for dedupe. */
const MAX_TRACKED_EVENT_IDS = 64;
/** How long a fragment ("So tell me about") waits for the rest of its question (LIVE-010). */
export const COALESCE_WINDOW_MS = 900;
/** Finals merged into one utterance at most, so a monologue is not held forever. */
const MAX_COALESCED_FINALS = 3;

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

// ── Surfacing gate (LIVE-009) ───────────────────────────────────────────────

/** A detection older than this is not worth answering any more (queue staleness). */
export const QUESTION_STALE_MS = 20_000;
/** Near-identical questions within this window surface once. */
export const SURFACE_DEDUPE_WINDOW_MS = 90_000;
/** After a suggestion, the same speaker's next generic question waits this long. */
export const SURFACE_COOLDOWN_MS = 8_000;
const DUPLICATE_SIMILARITY = 0.8;
/** Each dismissed live suggestion raises the bar a little; capped so it never mutes. */
const DISMISSAL_STEP = 0.05;
const MAX_DISMISSAL_PENALTY = 0.15;

/** Words that carry no question on their own ("okay?", "right?", "so, what?"). */
const FILLER_WORDS: ReadonlySet<string> = new Set(
  "so okay ok um umm uh uhh uhm er well like right yeah yes no hmm mm alright anyway actually and but sorry what huh really hello hey".split(
    " ",
  ),
);
/** Meta / back-channel checks about the call itself, never a question to answer. */
const META_PHRASES =
  /^(?:(?:so|okay|ok|um|uh|and|but|sorry|alright|well|hey|hi)\s+)*(?:can you (?:hear|see) (?:me|us|my screen|the screen)|(?:is|was) my (?:screen|audio|mic|microphone|video) (?:visible|working|ok|okay|clear)|(?:can|could) you repeat(?: that| the question)?|(?:does|did) (?:that|this|it) make sense|makes? sense|(?:is|was) (?:that|this) (?:right|clear|ok|okay)|sounds? good|any (?:other )?questions(?: so far)?|are you (?:there|still there|with me)|am i (?:audible|on mute|muted)|you know)(?:\s+(?:right|so far|now))?$/;

const GENERIC_TYPES: ReadonlySet<DetectedEvent["type"]> = new Set(["question", "follow_up"]);
/** A speaker restating what they just asked ("sorry, I mean…"): the newest wording wins. */
const CORRECTION_LEAD = /^(?:sorry|i mean|i meant|actually|rather|let me rephrase|scratch that|no,? wait)\b/i;

export type SurfaceVerdict =
  "surface" | "not_substantive" | "below_threshold" | "duplicate" | "cooldown" | "stale";

/** What the gate knows about the session so far. */
export interface SurfaceContext {
  mode: BlueyMode;
  now: number;
  /** Questions surfaced recently, newest last. */
  surfaced: readonly { text: string; speaker?: string; at: number }[];
  /** Live suggestions the user dismissed (Esc / Stop) since listening started. */
  dismissals: number;
}

function words(text: string): string[] {
  return text
    .toLowerCase()
    .replace(/[^\p{L}\p{N}'\s]/gu, " ")
    .split(/\s+/)
    .filter(Boolean);
}

/** Pure: token-set Jaccard similarity of two utterances. */
function similarity(a: string, b: string): number {
  const left = new Set(words(a));
  const right = new Set(words(b));
  if (left.size === 0 || right.size === 0) return 0;
  let shared = 0;
  for (const word of left) if (right.has(word)) shared += 1;
  return shared / (left.size + right.size - shared);
}

export function isStale(event: DetectedEvent, now: number): boolean {
  const at = Date.parse(event.detectedAt);
  return Number.isFinite(at) && now - at > QUESTION_STALE_MS;
}

/** Pure: back-channel and call-meta utterances ("right?", "can you hear me?"). */
export function isSubstantive(event: DetectedEvent): boolean {
  const all = words(event.text);
  if (META_PHRASES.test(all.join(" "))) return false;
  // A specific detection (objection, coding problem, …) carries meaning even when short.
  if (!GENERIC_TYPES.has(event.type)) return true;
  return all.filter((word) => !FILLER_WORDS.has(word)).length >= 2;
}

/**
 * Pure gate: should this detection open a (billed) suggestion? Adaptive rather than a
 * kill switch — conversational modes answer as before, other modes only direct
 * questions, repeats and back-channel are dropped, and dismissals raise the bar.
 */
export function shouldSurface(event: DetectedEvent, ctx: SurfaceContext): SurfaceVerdict {
  if (isStale(event, ctx.now)) return "stale";
  if (!isSubstantive(event)) return "not_substantive";
  // Other modes (General, Team Meeting, Lecture, custom) answer only direct questions.
  const conversational = conversationalMode(ctx.mode);
  if (!conversational && !DIRECT_QUESTION_TYPES.has(event.type)) return "below_threshold";
  const base = conversational ? CLASSIFIER_MIN_CONFIDENCE : DIRECT_QUESTION_MIN_CONFIDENCE;
  const penalty = Math.min(MAX_DISMISSAL_PENALTY, ctx.dismissals * DISMISSAL_STEP);
  if (event.confidence < base + penalty) return "below_threshold";
  const recent = ctx.surfaced.filter((entry) => ctx.now - entry.at <= SURFACE_DEDUPE_WINDOW_MS);
  if (recent.some((entry) => similarity(entry.text, event.text) >= DUPLICATE_SIMILARITY)) return "duplicate";
  const last = recent.at(-1);
  if (
    last &&
    GENERIC_TYPES.has(event.type) &&
    last.speaker === event.speaker &&
    !CORRECTION_LEAD.test(event.text.trim()) &&
    ctx.now - last.at < SURFACE_COOLDOWN_MS
  ) {
    return "cooldown";
  }
  return "surface";
}

/** The live suggestion streaming into the thread right now, if any. */
let liveHandle: CancelHandle | null = null;
/** Live suggestions the user dismissed since listening started (feeds the gate). */
let dismissals = 0;

/**
 * End the live suggestion (Esc, Stop, a manual ask, a prepared answer shown over it):
 * its stream stops and nothing it produced is saved (LIVE-001).
 */
export async function cancelLiveSuggestion(options: { dismissed?: boolean } = {}): Promise<void> {
  const handle = liveHandle;
  liveHandle = null;
  if (!handle) return;
  // Esc / Stop on a suggestion is a signal it was not wanted: the gate gets stricter.
  if (options.dismissed) dismissals += 1;
  try {
    await handle.cancel();
  } catch (error) {
    console.warn("[proactive] cancel failed", error);
  }
}

/** Unknown (not loaded yet) counts as visible, so nothing is dropped at boot. */
function hudVisible(): boolean {
  return usePanelStore.getState().state?.visible ?? true;
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
/** Log a Team Meeting / Lecture detection that needs no answer on the session timeline (MODE-006). */
async function recordNotable(event: DetectedEvent, mode: BlueyMode, settings: Settings): Promise<void> {
  const session = useSessionStore.getState().active;
  const entry = session ? notableEntry(event, mode, settings.privacy.storeTranscripts) : null;
  if (!session || !entry) return;
  try {
    // Rust publishes `session.event`, which appends it to the timeline store.
    await bluey.session.addEvent({ sessionId: session.id, ...entry });
  } catch (error) {
    console.warn("[proactive] could not log the detection", error);
  }
}

export function startProactiveLoop(): Unlisten {
  const seen = new Set<string>();
  let queued: DetectedEvent | null = null;
  /** The newest question detected while the HUD was hidden (live display only). */
  let deferred: { event: DetectedEvent; at: number } | null = null;
  let busy = false;
  /** Questions that opened a suggestion recently (dedupe + cooldown). */
  let surfaced: SurfaceContext["surfaced"] = [];
  /** A counterpart's fragment waiting for the rest of its question (LIVE-010). */
  let held: { segment: TranscriptSegment; ids: string[]; timer: ReturnType<typeof setTimeout> } | null = null;

  const remember = (id: string) => {
    seen.add(id);
    if (seen.size > MAX_TRACKED_EVENT_IDS) {
      const oldest = seen.values().next().value;
      if (oldest !== undefined) seen.delete(oldest);
    }
  };

  const prepareFor = async (event: DetectedEvent): Promise<void> => {
    busy = true;
    // What opened a suggestion feeds dedupe and the cooldown (LIVE-009).
    surfaced = [
      ...surfaced,
      { text: event.text, at: Date.now(), ...(event.speaker ? { speaker: event.speaker } : {}) },
    ];
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
        const suggestion: SuggestionMeta = {
          question: event.text,
          ...(event.speaker ? { speaker: event.speaker } : {}),
        };
        generation = chat.begin(event.text, event.text, {
          phase: "thinking",
          suggestion,
          request: {
            trigger: "detected_event",
            detectedEvent: event,
            promptLabel: event.text,
            captureScreen: false,
          },
        });
        useProactiveStore.getState().setLive(event.id);
        callbacks = {
          ...liveCallbacks(generation, event),
          onHandle: (handle) => {
            liveHandle = handle;
          },
        };
      }
      // The suggestion continues the thread: it sees the answers already given (LIVE-014).
      const previous = completedResponses(chat.turns);
      const response = await getEngine().prepare(
        {
          trigger: "detected_event",
          captureScreen: false,
          detectedEvent: event,
          mode,
          session: sessionState.active,
          settings,
          previousResponses: previous.length > 0 ? previous : undefined,
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
      liveHandle = null;
      useProactiveStore.getState().setPreparing(null);
      useProactiveStore.getState().setLive(null);
      const next = queued;
      queued = null;
      // A question that waited too long behind the previous answer is dropped (LIVE-009).
      if (next && !isStale(next, Date.now())) void prepareFor(next);
    }
  };

  const schedule = (event: DetectedEvent): void => {
    if (busy) {
      queued = event; // newest question wins; a stale one is not worth preparing
      return;
    }
    void prepareFor(event);
  };

  const onDetected = (event: DetectedEvent): void => {
    if (!isHudWindow() || !proactiveEnabled() || !event.requiresResponse) return;
    if (seen.has(event.id)) return;
    remember(event.id);
    const mode = activeMode();
    if (!mode) return;
    const now = Date.now();
    surfaced = surfaced.filter((entry) => now - entry.at <= SURFACE_DEDUPE_WINDOW_MS);
    if (shouldSurface(event, { mode, now, surfaced, dismissals }) !== "surface") return;
    if (useSettingsStore.getState().settings?.ai.suggestionDisplay === "live" && !hudVisible()) {
      // No billed live turn streams into a hidden HUD: keep the newest question and
      // prepare it once the HUD is shown, while it is still fresh (LIVE-012).
      deferred = { event, at: Date.now() };
      return;
    }
    schedule(event);
  };

  const offPanel = usePanelStore.subscribe((state, previous) => {
    if (!state.state?.visible || previous.state?.visible === true || !deferred) return;
    const { event, at } = deferred;
    deferred = null;
    if (Date.now() - at <= PREPARED_TTL_MS && proactiveEnabled()) schedule(event);
  });

  const offApp = useAppStore.subscribe((state, previous) => {
    const now = state.status;
    const before = previous.status;
    // Stop listening: a question still waiting is no longer worth answering (LIVE-017).
    if (before?.audioActive && now && !now.audioActive) {
      if (held) clearTimeout(held.timer);
      held = null;
      queued = null;
      deferred = null;
      dismissals = 0;
    }
    // Answers prepared for the previous mode were written for it: drop them (MODE-012).
    if (before && now && before.modeId !== now.modeId) {
      getEngine().clearPrepared();
      useChatStore.getState().setPrepared(null);
      useProactiveStore.getState().setPrepared(null);
    }
  });

  const classifyFinal = async (segment: TranscriptSegment, segmentIds: string[]): Promise<void> => {
    const settings = useSettingsStore.getState().settings;
    const mode = activeMode();
    if (!settings || !mode) return;
    const others = useTranscriptStore.getState().segments.filter((s) => !segmentIds.includes(s.id));
    try {
      // The engine emits `question.detected` itself when the event needs a response.
      const event = await getEngine().classify({
        segment,
        recent: recentSegments(others, CLASSIFY_WINDOW_SECONDS),
        mode,
        settings,
        ...(segmentIds.length > 1 ? { segmentIds } : {}),
      });
      if (event) await recordNotable(event, mode, settings);
    } catch (error) {
      console.warn("[proactive] classify failed", error);
    }
  };

  const releaseHeld = (): void => {
    if (!held) return;
    const { segment, ids, timer } = held;
    clearTimeout(timer);
    held = null;
    void classifyFinal(segment, ids);
  };

  const onFinal = (segment: TranscriptSegment): void => {
    if (!isHudWindow() || !proactiveEnabled()) return;
    const mode = activeMode();
    if (!mode) return;
    let merged = segment;
    let ids = [segment.id];
    const previous = held;
    if (
      previous &&
      previous.segment.source === segment.source &&
      previous.segment.speaker === segment.speaker
    ) {
      // The same voice went on within the window: classify the whole utterance once.
      clearTimeout(previous.timer);
      held = null;
      merged = {
        ...segment,
        text: `${previous.segment.text.trim()} ${segment.text.trim()}`,
        startTime: previous.segment.startTime,
      };
      ids = [...previous.ids, segment.id];
    } else {
      releaseHeld();
    }
    const counterpart = labelSpeaker(merged, mode).speaker !== "You";
    if (counterpart && ids.length < MAX_COALESCED_FINALS && isOpenFragment(merged.text)) {
      held = { segment: merged, ids, timer: setTimeout(releaseHeld, COALESCE_WINDOW_MS) };
      return;
    }
    void classifyFinal(merged, ids);
  };

  const offFinal = eventBus.on("transcript.final", onFinal);
  const offDetected = eventBus.on("question.detected", onDetected);
  return () => {
    offFinal();
    offDetected();
    offPanel();
    offApp();
    if (held) clearTimeout(held.timer);
    held = null;
    queued = null;
    deferred = null;
  };
}

/** Test helper. */
export function resetProactiveForTest(): void {
  liveHandle = null;
  dismissals = 0;
  useProactiveStore.setState({ preparedEventId: null, preparingEventId: null, liveEventId: null });
}
