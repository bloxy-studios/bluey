import type { BlueyMode, DetectedEvent, TranscriptSegment } from "@/lib/types";
import { labelSpeaker } from "@/transcript/speaker";

export interface TranscriptLine {
  id: string;
  speaker: string;
  /** 0..1 — how sure the speaker label is (never certain). */
  speakerConfidence: number;
  text: string;
  partial: boolean;
  /** The segment triggered a detected question / event. */
  detected: boolean;
}

/** Lines shown when the strip is collapsed / expanded. */
export const COLLAPSED_LINES = 1;
export const EXPANDED_LINES = 4;

function fallbackSpeaker(segment: TranscriptSegment): { speaker: string; confidence: number } {
  if (segment.speaker) return { speaker: segment.speaker, confidence: segment.speakerConfidence ?? 0.6 };
  return segment.source === "microphone"
    ? { speaker: "You", confidence: 0.95 }
    : { speaker: "Speaker", confidence: 0.5 };
}

/**
 * Pure: newest `limit` finalized segments (oldest first) followed by the
 * in-flight partials (one per source), with speaker labels resolved through
 * the mode heuristics.
 */
export function buildTranscriptLines(
  segments: TranscriptSegment[],
  partials: readonly TranscriptSegment[],
  questions: DetectedEvent[],
  mode: BlueyMode | undefined,
  limit: number,
): TranscriptLine[] {
  const detectedSegmentIds = new Set(questions.flatMap((q) => q.segmentIds));
  const toLine = (segment: TranscriptSegment, isPartial: boolean): TranscriptLine => {
    const label = mode ? labelSpeaker(segment, mode) : fallbackSpeaker(segment);
    return {
      id: segment.id,
      speaker: label.speaker,
      speakerConfidence: label.confidence,
      text: segment.text,
      partial: isPartial,
      detected: detectedSegmentIds.has(segment.id),
    };
  };

  const finals = segments.filter((s) => s.text.trim().length > 0);
  const partialLines = partials
    .filter((p) => p.text.trim().length > 0)
    .sort((a, b) => a.startTime - b.startTime)
    .slice(-limit)
    .map((p) => toLine(p, true));
  const finalLimit = Math.max(0, limit - partialLines.length);
  // `slice(-0)` would return everything — guard the "partials only" case explicitly.
  const lines = finalLimit > 0 ? finals.slice(-finalLimit).map((s) => toLine(s, false)) : [];
  return [...lines, ...partialLines];
}
