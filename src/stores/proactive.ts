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
import {
  toBlueyError,
  type BlueyError,
  type BlueyMode,
  type DetectedEvent,
  type Settings,
  type TranscriptSegment,
} from "@/lib/types";
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
import { useHudUiStore } from "./hudUiStore";
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
/** How long after a fragment was classified alone the same voice may still finish it (LIVE-010). */
export const CONTINUATION_WINDOW_MS = 3_000;
/** A fragment this short heard alone (and without "?") is too little to answer. */
const MUTED_FRAGMENT_MAX_WORDS = 5;
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

/** When a question was heard (0 when unparseable, so any dated question outranks it). */
function detectedTime(event: DetectedEvent): number {
  const at = Date.parse(event.detectedAt);
  return Number.isFinite(at) ? at : 0;
}

function overlaps(a: readonly string[], b: readonly string[]): boolean {
  return a.some((id) => b.includes(id));
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
  /** A fragment classified on its own, which the same voice may still continue (LIVE-010). */
  let released: { segment: TranscriptSegment; ids: string[]; at: number; eventId?: string } | null = null;
  /** The continuation of `released`: its detection replaces the fragment's suggestion. */
  let correction: { segmentIds: readonly string[]; fragmentEventId?: string } | null = null;
  /** Segments of the latest short fragment classified alone: listed, never answered. */
  let mutedSegmentIds: ReadonlySet<string> = new Set();
  /** The question being prepared and whether its answer has started streaming. */
  let inflight: { event: DetectedEvent; streaming: boolean } | null = null;

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
    inflight = { event, streaming: false };
    let generation: number | null = null;
    try {
      const settings = useSettingsStore.getState().settings;
      const mode = activeMode();
      if (!settings || !mode) return;
      const sessionState = useSessionStore.getState();
      const chat = useChatStore.getState();
      const live = canShowLive(settings, chat.phase);
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
        const base = liveCallbacks(generation, event);
        callbacks = {
          ...base,
          onPhase: (phase, requestId) => {
            // Its first token is out: a newer question now waits instead of replacing it.
            if (phase === "streaming" && inflight?.event.id === event.id)
              inflight = { event, streaming: true };
            base.onPhase?.(phase, requestId);
          },
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
          screenAllowed: useHudUiStore.getState().screenEnabled,
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
      // The live turn opened for it must end with the error, not spin forever.
      const current = useChatStore.getState();
      if (
        generation !== null &&
        current.generation === generation &&
        current.turns.at(-1)?.status === "streaming"
      ) {
        current.fail(generation, toBlueyError(error, "ai"));
      }
    } finally {
      busy = false;
      liveHandle = null;
      inflight = null;
      useProactiveStore.getState().setPreparing(null);
      useProactiveStore.getState().setLive(null);
      const next = queued;
      queued = null;
      // Runnable only now: its wait behind the previous answer does not count against it
      // (LIVE-009 judges staleness when a question is heard, not while it queues).
      if (next) void prepareFor(next);
    }
  };

  const schedule = (event: DetectedEvent): void => {
    if (busy) {
      // The question asked last wins the queue — by when it was heard, not when its
      // detection arrived (a slower classification must not overtake a newer question).
      if (!queued || detectedTime(event) >= detectedTime(queued)) queued = event;
      // A live suggestion still waiting for its first token is not worth finishing:
      // the newer question starts now instead of after it (the finally runs the queue).
      if (queued === event && inflight && !inflight.streaming && liveHandle) void cancelLiveSuggestion();
      return;
    }
    void prepareFor(event);
  };

  /** A replaced or refuted question's suggestion goes: waiting, deferred or streaming. */
  const withdraw = (eventId: string): void => {
    if (queued?.id === eventId) queued = null;
    if (deferred?.event.id === eventId) deferred = null;
    if (inflight?.event.id === eventId) void cancelLiveSuggestion();
  };

  const onDetected = (event: DetectedEvent): void => {
    if (!isHudWindow() || !proactiveEnabled() || !event.requiresResponse) return;
    if (seen.has(event.id)) return;
    remember(event.id);
    if (released && overlaps(event.segmentIds, released.ids)) released = { ...released, eventId: event.id };
    // A short fragment heard alone ("So tell me about") is listed, never answered (LIVE-010).
    if (event.segmentIds.length > 0 && event.segmentIds.every((id) => mutedSegmentIds.has(id))) return;
    const mode = activeMode();
    if (!mode) return;
    const now = Date.now();
    surfaced = surfaced.filter((entry) => now - entry.at <= SURFACE_DEDUPE_WINDOW_MS);
    const replaces = correction && overlaps(event.segmentIds, correction.segmentIds) ? correction : null;
    if (replaces) correction = null;
    const verdict = shouldSurface(event, { mode, now, surfaced, dismissals });
    // The rest of a question split by a pause corrects the fragment: the fragment's own
    // suggestion must not hold it off as a repeat or through the speaker cooldown.
    const corrects = replaces !== null && (verdict === "cooldown" || verdict === "duplicate");
    if (verdict !== "surface" && !corrects) return;
    if (replaces?.fragmentEventId) withdraw(replaces.fragmentEventId);
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
    // Shown too late, the conversation has moved past it: the queued lane's rule (LIVE-009).
    const now = Date.now();
    if (now - at <= PREPARED_TTL_MS && !isStale(event, now) && proactiveEnabled()) schedule(event);
  });

  const offApp = useAppStore.subscribe((state, previous) => {
    const now = state.status;
    const before = previous.status;
    // Stop listening: a question still waiting is no longer worth answering (LIVE-017).
    if (before?.audioActive && now && !now.audioActive) {
      if (held) clearTimeout(held.timer);
      held = null;
      released = null;
      correction = null;
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
        onRefuted: (refuted) => {
          // Not a question after all: its suggestion goes and it puts no one on cooldown.
          withdraw(refuted.id);
          surfaced = surfaced.filter((entry) => entry.text !== refuted.text);
        },
      });
      if (event) await recordNotable(event, mode, settings);
    } catch (error) {
      console.warn("[proactive] classify failed", error);
    }
  };

  /** `afterPause`: the window ran out, so the same voice may still finish the question (LIVE-010). */
  const releaseHeld = (afterPause = false): void => {
    if (!held) return;
    const { segment, ids, timer } = held;
    clearTimeout(timer);
    held = null;
    // Another voice speaking ends the turn: only a fragment cut by a pause is continued.
    released = afterPause ? { segment, ids, at: Date.now() } : null;
    if (segment.text.trim().split(/\s+/).length <= MUTED_FRAGMENT_MAX_WORDS) mutedSegmentIds = new Set(ids);
    void classifyFinal(segment, ids);
  };

  const sameVoice = (a: TranscriptSegment, b: TranscriptSegment): boolean =>
    a.source === b.source && a.speaker === b.speaker;

  const onFinal = (segment: TranscriptSegment): void => {
    if (!isHudWindow() || !proactiveEnabled()) return;
    const mode = activeMode();
    if (!mode) return;
    let merged = segment;
    let ids = [segment.id];
    const previous = held;
    const fragment = released;
    released = null;
    if (previous && sameVoice(previous.segment, segment)) {
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
      if (
        fragment &&
        sameVoice(fragment.segment, segment) &&
        Date.now() - fragment.at <= CONTINUATION_WINDOW_MS
      ) {
        // The rest arrived after the fragment was classified alone: the whole question
        // replaces the fragment's suggestion instead of losing to its cooldown.
        merged = {
          ...segment,
          text: `${fragment.segment.text.trim()} ${segment.text.trim()}`,
          startTime: fragment.segment.startTime,
        };
        ids = [...fragment.ids, segment.id];
        correction = { segmentIds: ids, ...(fragment.eventId ? { fragmentEventId: fragment.eventId } : {}) };
      }
    }
    const counterpart = labelSpeaker(merged, mode).speaker !== "You";
    if (counterpart && ids.length < MAX_COALESCED_FINALS && isOpenFragment(merged.text)) {
      held = { segment: merged, ids, timer: setTimeout(() => releaseHeld(true), COALESCE_WINDOW_MS) };
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
