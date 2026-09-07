/**
 * Speaker labelling: heuristic only, never certain.
 * Microphone audio is the user ("You", 0.95). System audio is the other party,
 * named by mode ("Interviewer" / "Customer" / "Candidate" / "Speaker", 0.6).
 */

import type { BlueyMode, TranscriptSegment } from "@/lib/types";
import { isCandidateMode } from "@/modes/registry";

export interface SpeakerLabel {
  speaker: string;
  confidence: number;
}

export function counterpartLabelFor(mode: BlueyMode): string {
  if (mode.id === "sales" || mode.responseSchema === "sales") return "Customer";
  if (mode.id === "recruiting" || mode.responseSchema === "recruiting") return "Candidate";
  if (isCandidateMode(mode)) return "Interviewer";
  if (mode.responseSchema === "lecture") return "Lecturer";
  return "Speaker";
}

/**
 * Label a segment's speaker. An existing label with higher confidence than
 * our heuristic is kept.
 */
export function labelSpeaker(segment: TranscriptSegment, mode: BlueyMode): SpeakerLabel {
  const heuristic: SpeakerLabel =
    segment.source === "microphone"
      ? { speaker: "You", confidence: 0.95 }
      : { speaker: counterpartLabelFor(mode), confidence: 0.6 };

  if (
    segment.speaker &&
    segment.speakerConfidence !== undefined &&
    segment.speakerConfidence > heuristic.confidence
  ) {
    return { speaker: segment.speaker, confidence: Math.min(segment.speakerConfidence, 0.99) };
  }
  return heuristic;
}
