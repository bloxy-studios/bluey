import { create } from "zustand";

import { PREPARED_TTL_MS, type AskTrigger, type EnginePhase } from "@/lib/engine-contract";
import type { BlueyError, BlueyResponse, DetectedEvent } from "@/lib/types";
import { createId } from "@/lib/utils/id";

export type TurnStatus = "streaming" | "done" | "error" | "cancelled";

/** Provenance of a suggestion Bluey opened itself: the question it answers and who asked it. */
export interface SuggestionMeta {
  question: string;
  speaker?: string;
}

/** What was asked, kept on the turn so Retry/Regenerate re-send the same request (UX-011). */
export interface TurnRequest {
  trigger: AskTrigger;
  instruction?: string;
  captureScreen?: boolean;
  promptLabel?: string;
  detectedEvent?: DetectedEvent;
}

export interface ChatTurn {
  id: string;
  /** The user's prompt; undefined for context-only asks ("Assist"). */
  prompt?: string;
  /** Label shown in the prompt pill when there is no typed prompt. */
  promptLabel: string;
  /** Set on turns Bluey opened for a detected question (rendered as a suggestion, not a prompt). */
  suggestion?: SuggestionMeta;
  response: BlueyResponse | null;
  status: TurnStatus;
  error?: BlueyError;
  /** The request that produced this turn (absent on answers shown from the prepared cache). */
  request?: TurnRequest;
}

export interface BeginOptions {
  /** Initial phase; a suggestion never reads the screen, so it starts at `thinking`. */
  phase?: EnginePhase;
  suggestion?: SuggestionMeta;
  request?: TurnRequest;
}

export interface ShowOptions {
  promptLabel?: string;
  suggestion?: SuggestionMeta;
  request?: TurnRequest;
}

interface ChatStore {
  turns: ChatTurn[];
  /**
   * Monotonic generation counter: every `begin` bumps it and all draft/apply
   * methods ignore calls carrying an older generation — a stale stream can
   * never overwrite a newer one.
   */
  generation: number;
  phase: EnginePhase | null;
  activeRequestId: string | null;
  /** Proactively prepared response (from `response.prepared`) awaiting ⌘⇧↵. */
  prepared: BlueyResponse | null;

  begin(prompt: string | undefined, promptLabel?: string, options?: BeginOptions): number;
  setActiveRequest(generation: number, requestId: string): void;
  setPhase(generation: number, phase: EnginePhase): void;
  /** Queue a streamed draft; the newest one lands in `turns` at most once per frame (PERF-003). */
  applyDraft(generation: number, response: BlueyResponse): void;
  /** Write the queued draft now (terminal transitions call it so no text is lost). */
  flushDraft(): void;
  complete(generation: number, response: BlueyResponse): void;
  fail(generation: number, error: BlueyError): void;
  markCancelled(generation: number): void;
  /** Show a finished response directly (prepared responses taken via ⌘⇧↵). */
  showResponse(response: BlueyResponse, options?: ShowOptions): void;
  setPrepared(response: BlueyResponse | null): void;
  newChat(): void;
}

function updateLast(turns: ChatTurn[], update: (turn: ChatTurn) => ChatTurn): ChatTurn[] {
  if (turns.length === 0) return turns;
  const last = turns[turns.length - 1];
  if (!last) return turns;
  return [...turns.slice(0, -1), update(last)];
}

/** A response on screen is no longer "prepared and waiting". */
export function shownResponse(response: BlueyResponse): BlueyResponse {
  if (!response.prepared) return response;
  const { prepared: _prepared, ...shown } = response;
  return shown;
}

let preparedExpiry: ReturnType<typeof setTimeout> | null = null;

/**
 * The newest streamed draft not yet in `turns`. A stream delivers a draft per
 * token; writing each one re-rendered the whole thread, so drafts are coalesced
 * to one store write per animation frame (PERF-003). The frame is requested
 * before the engine's first-paint stamp, so that stamp runs after the commit.
 */
let pendingDraft: { generation: number; response: BlueyResponse } | null = null;
let draftFrameScheduled = false;

function onNextFrame(callback: () => void): void {
  if (typeof requestAnimationFrame === "function") requestAnimationFrame(callback);
  else setTimeout(callback, 16);
}

export const useChatStore = create<ChatStore>((set, get) => ({
  turns: [],
  generation: 0,
  phase: null,
  activeRequestId: null,
  prepared: null,

  begin: (prompt, promptLabel, options = {}) => {
    // The superseded turn keeps the text it had streamed so far.
    get().flushDraft();
    const generation = get().generation + 1;
    set((state) => ({
      generation,
      phase: options.phase ?? "capturing",
      turns: [
        ...state.turns.map((t) => (t.status === "streaming" ? { ...t, status: "cancelled" as const } : t)),
        {
          id: createId("turn"),
          prompt,
          promptLabel: promptLabel ?? prompt ?? "Assist",
          ...(options.suggestion ? { suggestion: options.suggestion } : {}),
          ...(options.request ? { request: options.request } : {}),
          response: null,
          status: "streaming" as const,
        },
      ],
    }));
    return generation;
  },

  setActiveRequest: (generation, requestId) => {
    if (generation !== get().generation) return;
    set({ activeRequestId: requestId });
  },

  setPhase: (generation, phase) => {
    if (generation !== get().generation) return;
    set({ phase });
  },

  applyDraft: (generation, response) => {
    if (generation !== get().generation) return;
    pendingDraft = { generation, response };
    if (draftFrameScheduled) return;
    draftFrameScheduled = true;
    onNextFrame(() => {
      draftFrameScheduled = false;
      get().flushDraft();
    });
  },

  flushDraft: () => {
    const draft = pendingDraft;
    pendingDraft = null;
    if (!draft || draft.generation !== get().generation) return;
    set((state) => ({ turns: updateLast(state.turns, (turn) => ({ ...turn, response: draft.response })) }));
  },

  complete: (generation, response) => {
    if (generation !== get().generation) return;
    // The final response supersedes any queued draft.
    pendingDraft = null;
    set((state) => ({
      phase: "done",
      activeRequestId: null,
      turns: updateLast(state.turns, (turn) => ({ ...turn, response, status: "done" as const })),
    }));
  },

  fail: (generation, error) => {
    if (generation !== get().generation) return;
    get().flushDraft();
    set((state) => ({
      phase: "error",
      activeRequestId: null,
      turns: updateLast(state.turns, (turn) => ({ ...turn, error, status: "error" as const })),
    }));
  },

  markCancelled: (generation) => {
    if (generation !== get().generation) return;
    get().flushDraft();
    set((state) => ({
      phase: "cancelled",
      activeRequestId: null,
      turns: updateLast(state.turns, (turn) =>
        turn.status === "streaming" ? { ...turn, status: "cancelled" as const } : turn,
      ),
    }));
  },

  showResponse: (response, options = {}) => {
    const shown = shownResponse(response);
    get().flushDraft();
    set((state) => ({
      generation: state.generation + 1,
      phase: "done",
      activeRequestId: null,
      prepared: state.prepared?.id === response.id ? null : state.prepared,
      turns: [
        // Like `begin`: a turn still streaming is superseded, never left spinning (LIVE-011).
        ...state.turns.map((t) => (t.status === "streaming" ? { ...t, status: "cancelled" as const } : t)),
        {
          id: createId("turn"),
          prompt: shown.prompt,
          promptLabel: options.promptLabel ?? shown.prompt ?? "Suggestion",
          ...(options.suggestion ? { suggestion: options.suggestion } : {}),
          ...(options.request ? { request: options.request } : {}),
          response: shown,
          status: "done" as const,
        },
      ],
    }));
  },

  setPrepared: (response) => {
    if (preparedExpiry) clearTimeout(preparedExpiry);
    preparedExpiry = null;
    set({ prepared: response });
    if (!response) return;
    // The hint must not outlive the question: after the TTL, ⌘⇧↵ generates afresh (LIVE-016).
    preparedExpiry = setTimeout(() => {
      preparedExpiry = null;
      if (get().prepared?.id === response.id) set({ prepared: null });
    }, PREPARED_TTL_MS);
  },

  newChat: () => {
    pendingDraft = null;
    set((state) => ({
      turns: [],
      phase: null,
      activeRequestId: null,
      prepared: null,
      generation: state.generation + 1,
    }));
  },
}));

/** Latest completed responses, oldest first (for follow-up context). */
export function completedResponses(turns: ChatTurn[]): BlueyResponse[] {
  return turns.filter((t) => t.status === "done" && t.response).map((t) => t.response as BlueyResponse);
}
