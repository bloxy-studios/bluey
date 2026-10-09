/**
 * Team Meeting and Lecture detections that need no answer — a decision, an
 * action item, an important statement, a topic change — belong on the session
 * timeline, where the summary reads them (MODE-006). This maps a detection to
 * the timeline entry it becomes; the proactive loop writes it.
 */
import type { BlueyMode, DetectedEvent, DetectedEventType, SessionEventType } from "@/lib/types";

interface NotableKind {
  type: SessionEventType;
  title: string;
}

const NOTABLE_KINDS: Partial<Record<DetectedEventType, NotableKind>> = {
  decision: { type: "decision_detected", title: "Decision" },
  action_item: { type: "action_item_detected", title: "Action item" },
  important_statement: { type: "important_statement", title: "Important" },
  topic_change: { type: "topic_change", title: "Topic change" },
};

/** Which detections each response schema keeps (the modes that advertise them). */
const NOTABLE_BY_SCHEMA: Partial<Record<BlueyMode["responseSchema"], ReadonlySet<DetectedEventType>>> = {
  meeting: new Set(["decision", "action_item", "important_statement", "topic_change"]),
  lecture: new Set(["important_statement", "topic_change"]),
};

export interface NotableEntry {
  type: SessionEventType;
  title: string;
  /** The spoken words — only when the user keeps transcripts (Privacy → Store transcripts). */
  detail?: string;
  refs: Record<string, string>;
  confidence: number;
}

/** The timeline entry for `event` in `mode`, or null when it is not a notable, unanswered detection. */
export function notableEntry(
  event: DetectedEvent,
  mode: BlueyMode,
  storeTranscripts: boolean,
): NotableEntry | null {
  if (event.requiresResponse) return null;
  const kind = NOTABLE_KINDS[event.type];
  if (!kind || !NOTABLE_BY_SCHEMA[mode.responseSchema]?.has(event.type)) return null;
  const segmentId = event.segmentIds[0];
  return {
    type: kind.type,
    title: kind.title,
    ...(storeTranscripts ? { detail: event.text } : {}),
    refs: segmentId ? { segmentId } : {},
    confidence: event.confidence,
  };
}
