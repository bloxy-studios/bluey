import { create } from "zustand";

import type { EnginePhase } from "@/lib/engine-contract";
import type { BlueyError, BlueyResponse } from "@/lib/types";
import { createId } from "@/lib/utils/id";

export type TurnStatus = "streaming" | "done" | "error" | "cancelled";

export interface ChatTurn {
  id: string;
  /** The user's prompt; undefined for context-only asks ("Assist"). */
  prompt?: string;
  /** Label shown in the prompt pill when there is no typed prompt. */
  promptLabel: string;
  response: BlueyResponse | null;
  status: TurnStatus;
  error?: BlueyError;
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

  begin(prompt: string | undefined, promptLabel?: string): number;
  setActiveRequest(generation: number, requestId: string): void;
  setPhase(generation: number, phase: EnginePhase): void;
  applyDraft(generation: number, response: BlueyResponse): void;
  complete(generation: number, response: BlueyResponse): void;
  fail(generation: number, error: BlueyError): void;
  markCancelled(generation: number): void;
  /** Show a response directly (prepared responses taken via ⌘⇧↵). */
  showResponse(response: BlueyResponse, promptLabel?: string): void;
  setPrepared(response: BlueyResponse | null): void;
  newChat(): void;
}

function updateLast(turns: ChatTurn[], update: (turn: ChatTurn) => ChatTurn): ChatTurn[] {
  if (turns.length === 0) return turns;
  const last = turns[turns.length - 1];
  if (!last) return turns;
  return [...turns.slice(0, -1), update(last)];
}

export const useChatStore = create<ChatStore>((set, get) => ({
  turns: [],
  generation: 0,
  phase: null,
  activeRequestId: null,
  prepared: null,

  begin: (prompt, promptLabel) => {
    const generation = get().generation + 1;
    set((state) => ({
      generation,
      phase: "capturing",
      turns: [
        ...state.turns.map((t) => (t.status === "streaming" ? { ...t, status: "cancelled" as const } : t)),
        {
          id: createId("turn"),
          prompt,
          promptLabel: promptLabel ?? prompt ?? "Assist",
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

  showResponse: (response, promptLabel) => {
    set((state) => ({
      generation: state.generation + 1,
      phase: "done",
      activeRequestId: null,
      prepared: state.prepared?.id === response.id ? null : state.prepared,
      turns: [
        ...state.turns,
        {
          id: createId("turn"),
          prompt: response.prompt,
          promptLabel: promptLabel ?? response.prompt ?? "Suggestion",
          response,
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
