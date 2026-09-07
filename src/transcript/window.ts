/**
 * Rolling transcript window helpers: recency filtering, partial dedupe, and a
 * cheap extractive summary hook for the transcript that scrolled out of the
 * window.
 */

import type { TranscriptSegment } from "@/lib/types";

/**
 * Segments inside the window ending at `nowMs` (defaults to the newest
 * segment end): ended after the cutoff and started at or before the
 * reference. Ordering preserved.
 */
export function recentSegments(
  segments: TranscriptSegment[],
  seconds: number,
  nowMs?: number,
): TranscriptSegment[] {
  if (segments.length === 0) return [];
  const reference = nowMs ?? segments.reduce((max, s) => Math.max(max, s.endTime), 0);
  const cutoff = reference - seconds * 1000;
  return segments.filter((segment) => segment.endTime >= cutoff && segment.startTime <= reference);
}

/**
 * Drop stale partials: a non-finalized segment is superseded by any later
 * segment from the same source that overlaps its time range, and by a
 * finalized segment with the same id. The latest partial per source survives.
 */
export function dedupePartials(segments: TranscriptSegment[]): TranscriptSegment[] {
  const finalizedIds = new Set(segments.filter((s) => s.finalized).map((s) => s.id));
  const result: TranscriptSegment[] = [];

  for (let i = 0; i < segments.length; i += 1) {
    const segment = segments[i];
    if (!segment) continue;
    if (segment.finalized) {
      result.push(segment);
      continue;
    }
    if (finalizedIds.has(segment.id)) continue; // finalized version exists
    // Superseded by a later same-source segment overlapping its window?
    let superseded = false;
    for (let j = i + 1; j < segments.length; j += 1) {
      const later = segments[j];
      if (!later || later.source !== segment.source) continue;
      const overlaps = later.startTime <= segment.endTime && later.endTime >= segment.startTime;
      if (overlaps) {
        superseded = true;
        break;
      }
    }
    if (!superseded) result.push(segment);
  }
  return result;
}

export type OlderTranscriptSummarizer = (segments: TranscriptSegment[]) => string;

/**
 * Cheap extractive summary of older transcript: keeps the first sentence of
 * each speaker turn, capped. Used as the default `summarizeOlder` hook; a
 * model-based summarizer can be injected instead.
 */
export const summarizeOlder: OlderTranscriptSummarizer = (segments) => {
  if (segments.length === 0) return "";
  const lines: string[] = [];
  let lastSpeaker: string | undefined;
  for (const segment of segments) {
    const speaker = segment.speaker ?? (segment.source === "microphone" ? "You" : "Speaker");
    if (speaker === lastSpeaker) continue; // one line per turn
    lastSpeaker = speaker;
    const sentence = segment.text.split(/(?<=[.!?])\s/)[0] ?? segment.text;
    lines.push(`${speaker}: ${sentence.slice(0, 140)}`);
    if (lines.length >= 12) break;
  }
  return lines.join("\n");
};
