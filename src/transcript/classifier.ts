/**
 * Heuristic transcript classifier: detects questions, behavioral prompts,
 * coding problems, sales objections/buying signals, meeting decisions/action
 * items and lecture definitions from a finalized segment.
 *
 * Pure and cheap — runs on every finalized segment. The engine optionally
 * refines mid-confidence results (0.4–0.7) with a fast model.
 */

import type { BlueyMode, DetectedEvent, DetectedEventType, TranscriptSegment } from "@/lib/types";
import { isCandidateMode } from "@/modes/registry";
import { labelSpeaker } from "./speaker";

export interface ClassifySegmentArgs {
  segment: TranscriptSegment;
  recent: TranscriptSegment[];
  mode: BlueyMode;
  /** Competitor names to watch for (from mode docs), optional. */
  competitorNames?: string[];
  now?: () => Date;
  idGen?: () => string;
}

interface Candidate {
  type: DetectedEventType;
  confidence: number;
}

// ── Cue patterns ────────────────────────────────────────────────────────────

const INTERROGATIVE_LEAD =
  /^(what|how|why|when|where|who|which|can|could|would|will|should|do|does|did|is|are|have you|has|tell me|walk me|talk me|describe|explain)\b/i;
const RISING_PATTERNS = /\b(tell me about|walk me through|talk me through|how would you|what would you|can you (explain|describe|tell)|give me an example)\b/i;

const BEHAVIORAL_MARKERS =
  /\b(tell me about a time|describe a (time|situation)|give (me |us )?an example of (a time|when)|walk me through a (time|situation)|a time (when|where) you|have you ever (had|faced|dealt))\b/i;

const CODING_MARKERS =
  /\b(implement|write (a|the) (function|method|program|algorithm|query)|code (this|it|up)|leetcode|time complexity|space complexity|big[- ]o|reverse a|traverse|two[- ]sum|binary (tree|search)|linked list|dynamic programming)\b/i;

const SYSTEM_DESIGN_MARKERS =
  /\b(design (a|an|the) (system|service|url shortener|rate limiter|news ?feed|chat (app|system)|notification|cache)|how would you (design|architect|scale)|system design)\b/i;

const TECHNICAL_MARKERS =
  /\b(architecture|database|index(es|ing)?|api|latency|throughput|concurrency|thread|memory|garbage collect|http|rest|graphql|sql|transaction)\b/i;

const OBJECTION_MARKERS =
  /\b(too expensive|too pricey|(not|n't) sure (about|if|that)|we already (use|have)|already working with|don'?t see the value|not convinced|why should we|(n't|not) in (the|our) budget|over budget|need to think about it|not a priority)\b/i;

const PRICING_CONCERN_MARKERS =
  /\b(cost(s)? too much|can'?t afford|cheaper|discount|price is (high|steep)|what does it cost)\b/i;

const BUYING_SIGNAL_MARKERS =
  /\b(how soon|how quickly|when (can|could) we (start|get)|pricing|price list|what.{0,12}cost|contract|trial|pilot|onboard(ing)?|next steps?|send (me|us) (a|the) (quote|proposal)|sign(ing)? up)\b/i;

const DECISION_MARKERS =
  /\b(let'?s go with|we (decided|agreed|are going) (on|with|to)|decision is|we'?ll (proceed|move forward) with|final call|settled on|approved)\b/i;

const ACTION_ITEM_MARKERS =
  /\b(i'?ll (take|do|own|handle|send|write|set up|follow up)|can you (take|do|own|handle|send|write|set up|follow up)|you'?ll (own|handle)|action item|to[- ]do|follow(ing)? up|by (monday|tuesday|wednesday|thursday|friday|saturday|sunday|tomorrow|next week|end of (day|week|month)|eod|eow)|due (on|by))\b/i;

const TOPIC_CHANGE_MARKERS =
  /\b(moving on|next (topic|item|up)|let'?s (switch|move) to|switching gears|last (topic|item)|one more thing|before we wrap)\b/i;

const DEFINITION_MARKERS =
  /\b(is defined as|we define .{1,40} as|means that|refers to|is called|is known as|the definition of|in other words)\b/i;

const IMPORTANT_MARKERS =
  /\b(this (will be|is) on the (exam|test|quiz)|remember (this|that)|the key (point|idea|takeaway)|crucially|importantly|make sure you)\b/i;

export function isQuestionText(text: string): { question: boolean; confidence: number } {
  const trimmed = text.trim();
  if (trimmed.length < 2) return { question: false, confidence: 0 };
  if (trimmed.includes("?")) return { question: true, confidence: 0.82 };
  if (RISING_PATTERNS.test(trimmed)) return { question: true, confidence: 0.72 };
  if (INTERROGATIVE_LEAD.test(trimmed) && trimmed.split(/\s+/).length >= 3) {
    return { question: true, confidence: 0.58 };
  }
  return { question: false, confidence: 0 };
}

function modeFamilyBonus(type: DetectedEventType, mode: BlueyMode): number {
  const schema = mode.responseSchema;
  const salesFamily: DetectedEventType[] = ["objection", "buying_signal", "pricing_concern", "competitor_mention"];
  const meetingFamily: DetectedEventType[] = ["decision", "action_item", "topic_change"];
  const interviewFamily: DetectedEventType[] = ["behavioral_question", "technical_question", "coding_problem"];
  if (schema === "sales" && salesFamily.includes(type)) return 0.08;
  if (schema === "meeting" && meetingFamily.includes(type)) return 0.08;
  if (isCandidateMode(mode) && interviewFamily.includes(type)) return 0.08;
  if (schema === "lecture" && type === "important_statement") return 0.08;
  return 0;
}

const RESPONSE_WORTHY: ReadonlySet<DetectedEventType> = new Set([
  "question",
  "behavioral_question",
  "technical_question",
  "coding_problem",
  "objection",
  "buying_signal",
  "pricing_concern",
  "competitor_mention",
  "follow_up",
]);

function conversationalMode(mode: BlueyMode): boolean {
  return (
    isCandidateMode(mode) ||
    mode.responseSchema === "sales" ||
    mode.responseSchema === "recruiting" ||
    mode.responseSchema === "suggested-response"
  );
}

function detectCandidates(text: string, args: ClassifySegmentArgs): Candidate[] {
  const candidates: Candidate[] = [];
  const q = isQuestionText(text);

  if (BEHAVIORAL_MARKERS.test(text)) {
    candidates.push({ type: "behavioral_question", confidence: 0.86 });
  }
  if (SYSTEM_DESIGN_MARKERS.test(text)) {
    candidates.push({ type: "technical_question", confidence: 0.82 });
  }
  if (CODING_MARKERS.test(text)) {
    candidates.push({ type: "coding_problem", confidence: q.question ? 0.82 : 0.68 });
  } else if (q.question && TECHNICAL_MARKERS.test(text)) {
    candidates.push({ type: "technical_question", confidence: Math.min(0.85, q.confidence + 0.08) });
  }

  if (OBJECTION_MARKERS.test(text)) candidates.push({ type: "objection", confidence: 0.78 });
  if (PRICING_CONCERN_MARKERS.test(text)) candidates.push({ type: "pricing_concern", confidence: 0.72 });
  if (BUYING_SIGNAL_MARKERS.test(text) && !OBJECTION_MARKERS.test(text)) {
    candidates.push({ type: "buying_signal", confidence: 0.68 });
  }
  for (const competitor of args.competitorNames ?? []) {
    if (competitor.length > 1 && text.toLowerCase().includes(competitor.toLowerCase())) {
      candidates.push({ type: "competitor_mention", confidence: 0.75 });
      break;
    }
  }

  if (DECISION_MARKERS.test(text)) candidates.push({ type: "decision", confidence: 0.78 });
  if (ACTION_ITEM_MARKERS.test(text)) candidates.push({ type: "action_item", confidence: 0.7 });
  if (TOPIC_CHANGE_MARKERS.test(text)) candidates.push({ type: "topic_change", confidence: 0.6 });

  if (DEFINITION_MARKERS.test(text) || IMPORTANT_MARKERS.test(text)) {
    candidates.push({ type: "important_statement", confidence: 0.66 });
  }

  if (q.question) candidates.push({ type: "question", confidence: q.confidence });
  return candidates;
}

function isFollowUpQuestion(args: ClassifySegmentArgs, speaker: string): boolean {
  const { segment, recent, mode } = args;
  for (let i = recent.length - 1; i >= 0; i -= 1) {
    const prior = recent[i];
    if (!prior || prior.id === segment.id) continue;
    if (segment.startTime - prior.endTime > 20_000) break;
    const priorSpeaker = labelSpeaker(prior, mode).speaker;
    if (priorSpeaker !== speaker) continue;
    if (isQuestionText(prior.text).question) return true;
    break;
  }
  return false;
}

export const CLASSIFIER_MIN_CONFIDENCE = 0.5;

/**
 * Classify one segment. Returns null when nothing actionable was detected or
 * confidence is below threshold. Never classifies unfinalized partials.
 */
export function classifySegment(args: ClassifySegmentArgs): DetectedEvent | null {
  const { segment, mode } = args;
  const now = args.now ?? (() => new Date());
  const idGen = args.idGen ?? (() => crypto.randomUUID());

  if (!segment.finalized) return null;
  const text = segment.text.trim();
  if (text.length < 2) return null;

  const candidates = detectCandidates(text, args).map((candidate) => ({
    type: candidate.type,
    confidence: Math.min(0.98, candidate.confidence + modeFamilyBonus(candidate.type, mode)),
  }));
  if (candidates.length === 0) return null;

  // A specific detection (objection, decision, coding_problem, ...) beats the
  // generic "question" type even when the question score is nominally higher.
  const pick = (list: Candidate[]): Candidate | null =>
    list.reduce<Candidate | null>((acc, c) => (!acc || c.confidence > acc.confidence ? c : acc), null);
  const specific = pick(candidates.filter((c) => c.type !== "question"));
  const best =
    specific && specific.confidence >= CLASSIFIER_MIN_CONFIDENCE ? specific : pick(candidates);
  if (!best || best.confidence < CLASSIFIER_MIN_CONFIDENCE) return null;

  const { speaker } = labelSpeaker(segment, mode);

  let type = best.type;
  if (type === "question" && isFollowUpQuestion(args, speaker)) type = "follow_up";

  const requiresResponse =
    RESPONSE_WORTHY.has(type) && speaker !== "You" && conversationalMode(mode);

  return {
    id: `evt_${idGen()}`,
    type,
    confidence: best.confidence,
    requiresResponse,
    text,
    segmentIds: [segment.id],
    speaker,
    detectedAt: now().toISOString(),
  };
}
