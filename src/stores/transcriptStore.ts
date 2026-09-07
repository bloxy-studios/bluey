import { create } from "zustand";

import type { DetectedEvent, TranscriptSegment } from "@/lib/types";

const MAX_SEGMENTS = 200;
const MAX_QUESTIONS = 12;

interface TranscriptStore {
  /** Finalized segments, ring-buffered. */
  segments: TranscriptSegment[];
  /** The in-flight partial segment (replaced on every `transcript.partial`). */
  partial: TranscriptSegment | null;
  /** Recently detected questions/events (newest last). */
  questions: DetectedEvent[];
  levels: { microphone: number; system: number };
  applyPartial(segment: TranscriptSegment): void;
  applyFinal(segment: TranscriptSegment): void;
  pushQuestion(event: DetectedEvent): void;
  consumeQuestion(id: string): void;
  setLevels(levels: { microphone: number; system: number }): void;
  clear(sessionId?: string): void;
}

export const useTranscriptStore = create<TranscriptStore>((set) => ({
  segments: [],
  partial: null,
  questions: [],
  levels: { microphone: 0, system: 0 },
  applyPartial: (segment) => set({ partial: segment }),
  applyFinal: (segment) =>
    set((state) => ({
      partial: state.partial?.id === segment.id ? null : state.partial,
      segments: [...state.segments.filter((s) => s.id !== segment.id), segment].slice(-MAX_SEGMENTS),
    })),
  pushQuestion: (event) =>
    set((state) => ({ questions: [...state.questions, event].slice(-MAX_QUESTIONS) })),
  consumeQuestion: (id) => set((state) => ({ questions: state.questions.filter((q) => q.id !== id) })),
  setLevels: (levels) => set({ levels }),
  clear: (sessionId) =>
    set((state) => ({
      partial: null,
      segments: sessionId ? state.segments.filter((s) => s.sessionId !== sessionId) : [],
    })),
}));
