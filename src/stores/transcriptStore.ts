import { create } from "zustand";

import type { AudioSource, DetectedEvent, TranscriptSegment } from "@/lib/types";

const MAX_SEGMENTS = 200;
const MAX_QUESTIONS = 12;

interface TranscriptStore {
  /** Finalized segments, ring-buffered. */
  segments: TranscriptSegment[];
  /**
   * The in-flight partial per source (replaced on every `transcript.partial`),
   * so the microphone and system audio never overwrite each other's line.
   */
  partials: Partial<Record<AudioSource, TranscriptSegment>>;
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
  partials: {},
  questions: [],
  levels: { microphone: 0, system: 0 },
  applyPartial: (segment) => set((state) => ({ partials: { ...state.partials, [segment.source]: segment } })),
  // A source streams one utterance at a time: its final supersedes its partial
  // even when an older helper keyed the two differently.
  applyFinal: (segment) =>
    set((state) => ({
      partials: withoutSource(state.partials, segment.source),
      segments: [...state.segments.filter((s) => s.id !== segment.id), segment].slice(-MAX_SEGMENTS),
    })),
  pushQuestion: (event) =>
    set((state) => ({ questions: [...state.questions, event].slice(-MAX_QUESTIONS) })),
  consumeQuestion: (id) => set((state) => ({ questions: state.questions.filter((q) => q.id !== id) })),
  setLevels: (levels) => set({ levels }),
  clear: (sessionId) =>
    set((state) => ({
      partials: {},
      segments: sessionId ? state.segments.filter((s) => s.sessionId !== sessionId) : [],
    })),
}));

function withoutSource(
  partials: Partial<Record<AudioSource, TranscriptSegment>>,
  source: AudioSource,
): Partial<Record<AudioSource, TranscriptSegment>> {
  const { [source]: _done, ...rest } = partials;
  return rest;
}
