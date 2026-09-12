import { create } from "zustand";

import type { EnginePhase } from "@/lib/engine-contract";
import type { BlueyError, BlueyResponse } from "@/lib/types";
import { createId } from "@/lib/utils/id";

export type TurnStatus = "streaming" | "done" | "error" | "cancelled";

/** Provenance of a suggestion Bluey opened itself: the question it answers and who asked it. */
export interface SuggestionMeta {
  question: string;
  speaker?: string;
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
}

export interface BeginOptions {
  /** Initial phase; a suggestion never reads the screen, so it starts at `thinking`. */
  phase?: EnginePhase;
  suggestion?: SuggestionMeta;
}

export interface ShowOptions {
  promptLabel?: string;
  suggestion?: SuggestionMeta;
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
  applyDraft(generation: number, response: BlueyResponse): void;
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

export const useChatStore = create<ChatStore>((set, get) => ({
  turns: [],
  generation: 0,
  phase: null,
  activeRequestId: null,
  prepared: null,

  begin: (prompt, promptLabel, options = {}) => {
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
    set((state) => ({ turns: updateLast(state.turns, (turn) => ({ ...turn, response })) }));
  },

  complete: (generation, response) => {
    if (generation !== get().generation) return;
    set((state) => ({
      phase: "done",
      activeRequestId: null,
      turns: updateLast(state.turns, (turn) => ({ ...turn, response, status: "done" as const })),
    }));
  },

  fail: (generation, error) => {
    if (generation !== get().generation) return;
    set((state) => ({
      phase: "error",
      activeRequestId: null,
      turns: updateLast(state.turns, (turn) => ({ ...turn, error, status: "error" as const })),
    }));
  },

  markCancelled: (generation) => {
    if (generation !== get().generation) return;
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
    set((state) => ({
      generation: state.generation + 1,
      phase: "done",
      activeRequestId: null,
      prepared: state.prepared?.id === response.id ? null : state.prepared,
      turns: [
        ...state.turns,
        {
          id: createId("turn"),
          prompt: shown.prompt,
          promptLabel: options.promptLabel ?? shown.prompt ?? "Suggestion",
          ...(options.suggestion ? { suggestion: options.suggestion } : {}),
          response: shown,
          status: "done" as const,
        },
      ],
    }));
  },

  setPrepared: (response) => set({ prepared: response }),

  newChat: () =>
    set((state) => ({
      turns: [],
      phase: null,
      activeRequestId: null,
      prepared: null,
      generation: state.generation + 1,
    })),
}));

/** Latest completed responses, oldest first (for follow-up context). */
export function completedResponses(turns: ChatTurn[]): BlueyResponse[] {
  return turns.filter((t) => t.status === "done" && t.response).map((t) => t.response as BlueyResponse);
}
