/**
 * Intent classification for the current ask: which AI task is this, does it
 * need vision, how deep should the model think, which structured schema
 * should the response use — and which shape the answer takes (`AnswerShape`:
 * the option to pick, yes/no, the missing words, the better of two, …).
 */

import type { AskTrigger } from "@/lib/engine-contract";
import type {
  AITask,
  AnswerShape,
  BlueyMode,
  ContextSnapshot,
  DetectedEvent,
  LatencyBudget,
  ReasoningLevel,
  ResponseSchemaId,
  ResponseType,
} from "@/lib/types";
import { defaultTaskFor } from "@/modes/registry";
import { currentQuestionText } from "./fusion";

export interface IntentInput {
  instruction?: string;
  snapshot: ContextSnapshot;
  mode: BlueyMode;
  detectedEvent?: DetectedEvent;
  /** What started the ask; ⌘⇧↵ and detected questions default to the spoken shape. */
  trigger?: AskTrigger;
  /** Injectable clock for the "year >= current" research cue. */
  now?: () => Date;
}

export interface Intent {
  task: AITask;
  visionRequired: boolean;
  reasoning: ReasoningLevel;
  latency: LatencyBudget;
  responseType: ResponseType;
  schemaId: ResponseSchemaId;
  answerShape: AnswerShape;
}

const CODING_CUES =
  /\b(implement|write (a|the) (function|method|program|algorithm)|leetcode|time complexity|space complexity|big[- ]o|debug|fix (this|the) (bug|code|test)|refactor|unit test|regex|algorithm)\b/i;

const CODING_SCREEN_MARKERS =
  /(Example \d+:|Constraints:|Input:|Output:|function\s+\w+\s*\(|def\s+\w+\s*\(|class\s+\w+|Time Limit Exceeded|```)/;

const SYSTEM_DESIGN_CUES =
  /\b(design (a|an|the) (system|url shortener|rate limiter|feed|chat|notification|search|cache|scheduler|queue)|system design|high[- ]level (architecture|design)|scal(e|ability|ing)|shard|load balanc|distributed|architecture for|design .{0,40}\b(like|similar to)\b)\b/i;

const SUMMARIZE_CUES = /\b(summari[sz]e|recap|sum up|tl;?dr|key takeaways|meeting notes|wrap[- ]up)\b/i;

const RESEARCH_CUES = /\b(research|look up|latest|news|recent(ly)?|current(ly)?|deep dive|compare vendors?|competitors? of)\b/i;
const WHO_WHAT_IS = /\b(who|what)\s+(is|are|was|were)\s+([A-Z][\w.&-]*)/;
const URL_CUE = /https?:\/\/\S+/i;

const VISUAL_REFERENCE_CUES =
  /\b(this|that|the)\s+(chart|graph|diagram|screenshot|image|picture|figure|design|mockup|slide|table|plot)\b|\bon (my|the) screen\b|\bwhat am i looking at\b/i;

// ── Answer shapes ───────────────────────────────────────────────────────────
// Detection order (docs/MODE_SYSTEM.md › Answer shapes): assessment shapes
// that are explicit in the question or on the screen (compare, choice,
// fill-in, yes/no, calculation) beat the shapes implied by the task (code,
// design, summary), which beat the shapes implied by the trigger and the mode
// (written, spoken); explain / short answer are the defaults for open asks.

const COMPARE_CUES =
  /\b(which (response|answer|option|one|version|approach|of (these|the two|the following two)) is (better|best|stronger|more|preferable|correct|right))|\b(compare|contrast)\b|\b(evaluate|assess|judge|rate|rank|grade) (the |these |both |the two |two )?(responses?|answers?|options?|versions?|outputs?|candidates?|models?)\b|\b(better|stronger|preferred|best) (response|answer|option|version|output)\b|\bresponse [ab]\b|\bversus\b|\bvs\.?\s/i;
/** Two labelled candidates ("Response A" / "Response B", "Option 1" / "Option 2"). */
const PAIR_LABEL = /\b(response|answer|option|version|output|candidate|model)\s*(a|b|1|2|one|two)\b/gi;
const PAIR_VERB = /\b(better|best|prefer|preferred|evaluate|compare|rate|rank|judge|stronger|which)\b/i;

const CHOICE_CUES =
  /\b(which (of the following|one|option|statement|choice|answer)|select (the|one|all|two|three|every|each)|choose (the|one|all|two|three)|pick (the|one)|multiple[- ]choice|(correct|best|right|most (accurate|appropriate|likely)) (answer|option|choice|statement)|best describes|all that apply|which .{0,30}\b(true|false|correct|incorrect)\b)/i;
/** Lettered options — (A) / A. / a: — and the radio or checkbox glyphs OCR reads off a form. */
const OPTION_MARKER = /^\s*(?:\(?([A-Ea-e])[).:]|([☐☑○◯◻□●•▢◉]))\s+\S/gm;

const FILL_IN_CUES =
  /_{3,}|\[\s*blank\s*\]|\b(fill in the blanks?|complete the (sentence|statement|following|code|expression)|the missing (word|words|term|value|number|line))\b/i;
const FILL_IN_SCREEN_CUES = /\b(fill in the blanks?|complete the (sentence|statement|expression))\b/i;

const BOOLEAN_CUES = /\b(true or false|yes or no|correct or incorrect|valid or invalid)\b/i;
const YES_NO_OPENER = /^(is|are|was|were|do|does|did|can|could|should|would|will|has|have|had|am|shall|must)\b/i;

const CALCULATION_CUES =
  /\b(calculate|compute|how (many|much)|what is the (total|sum|difference|product|average|mean|median|mode|probability|percentage|percent|value|result|remainder|area|volume|distance|speed|rate)|solve for|evaluate the expression|round(ed)? to|to the nearest)\b|\d\s*[+\-*/×÷^=%]\s*\d/i;

const WRITTEN_CUES =
  /\b(write|draft|compose|rewrite|reword|rephrase|reply to|respond to)\b[^.?\n]{0,40}\b(email|e-mail|message|reply|comment|post|note|letter|essay|paragraph|statement|bio|text|dm|tweet|summary)\b|\b(cover letter|essay|email reply)\b/i;

const EXPLAIN_CUES =
  /\b(why|how (do|does|did|would|should|can|could|is|are|to|about)|explain|describe|walk (me )?through|what happens|difference between|what does .{1,40} mean|elaborate|in detail|tell me about)\b/i;
const SHORT_ANSWER_OPENER = /^(what|who|whom|when|where|which|name|define|state|list|give|identify|convert|translate)\b/i;

const SPOKEN_SCHEMAS: ReadonlySet<ResponseSchemaId> = new Set<ResponseSchemaId>([
  "suggested-response",
  "behavioral",
  "sales",
  "recruiting",
]);

/** Distinct lettered option markers, or radio/checkbox glyphs, at line starts. */
function distinctOptionMarkers(text: string): number {
  const letters = new Set<string>();
  let glyphs = 0;
  for (const match of text.matchAll(OPTION_MARKER)) {
    if (match[1]) letters.add(match[1].toUpperCase());
    else if (match[2]) glyphs += 1;
  }
  return Math.max(letters.size, glyphs);
}

/** "Response A … Response B" (same noun, two labels) next to a verb of judgement. */
function hasLabelledPair(text: string): boolean {
  const byNoun = new Map<string, Set<string>>();
  for (const match of text.matchAll(PAIR_LABEL)) {
    const noun = (match[1] ?? "").toLowerCase();
    const labels = byNoun.get(noun) ?? new Set<string>();
    labels.add((match[2] ?? "").toLowerCase());
    byNoun.set(noun, labels);
  }
  const paired = [...byNoun.values()].some((labels) => labels.size >= 2);
  return paired && PAIR_VERB.test(text);
}

export interface AssessmentShapeOptions {
  /** Spoken contexts (⌘⇧↵, detected questions, suggestion schemas) keep only compare and choice. */
  spoken?: boolean;
}

/**
 * Shapes that are explicit in the question or on the screen — a multiple
 * choice, a pair of responses to compare, a blank, a yes/no, a calculation.
 * Null when the ask is open.
 */
export function detectAssessmentShape(
  question: string,
  screenText: string,
  options: AssessmentShapeOptions = {},
): AnswerShape | null {
  const q = question.trim();
  const screen = screenText.slice(0, 6000);
  const both = `${q}\n${screen}`;
  if (COMPARE_CUES.test(q) || hasLabelledPair(both)) return "compare";
  if (CHOICE_CUES.test(q) || distinctOptionMarkers(both) >= 2) return "choice";
  if (options.spoken) return null;
  if (FILL_IN_CUES.test(q) || FILL_IN_SCREEN_CUES.test(screen)) return "fill_in";
  if (BOOLEAN_CUES.test(q)) return "boolean";
  if (YES_NO_OPENER.test(q) && q.length <= 160 && !EXPLAIN_CUES.test(q)) return "boolean";
  if (CALCULATION_CUES.test(q) && /\d/.test(both)) return "calculation";
  return null;
}

export interface AnswerShapeInput {
  question: string;
  screenText: string;
  task: AITask;
  schemaId: ResponseSchemaId;
  trigger?: AskTrigger;
}

/** The shape the answer should take (see the detection order above). */
export function detectAnswerShape(input: AnswerShapeInput): AnswerShape {
  const { screenText, task, schemaId, trigger } = input;
  const q = input.question.trim();
  const spoken =
    trigger === "shortcut_generate" || trigger === "detected_event" || SPOKEN_SCHEMAS.has(schemaId);
  const assessment = detectAssessmentShape(q, screenText, { spoken });
  if (assessment) return assessment;
  if (task === "coding") return "code";
  if (task === "system_design") return "design";
  if (task === "summarization") return "summary";
  if (WRITTEN_CUES.test(q)) return "written";
  if (spoken) return "spoken";
  if (EXPLAIN_CUES.test(q)) return "explain";
  if (q.length > 0 && q.length <= 120 && SHORT_ANSWER_OPENER.test(q)) return "short_answer";
  return "explain";
}

// ── Task / depth / schema ───────────────────────────────────────────────────

const LATENCY_ORDER: readonly LatencyBudget[] = ["ultra-fast", "fast", "balanced", "deep"];

function slowerOf(a: LatencyBudget, b: LatencyBudget): LatencyBudget {
  const result = LATENCY_ORDER[Math.max(LATENCY_ORDER.indexOf(a), LATENCY_ORDER.indexOf(b))];
  return result ?? "balanced";
}

function minimumLatencyFor(task: AITask): LatencyBudget {
  switch (task) {
    case "system_design":
    case "deep_reasoning":
      return "deep";
    case "coding":
    case "summarization":
    case "research":
      return "balanced";
    default:
      return "ultra-fast";
  }
}

function reasoningFor(task: AITask, mode: BlueyMode): ReasoningLevel {
  if (task === "system_design" || task === "deep_reasoning") return "deep";
  if (task === "coding" || task === "research" || task === "summarization") return "light";
  if (mode.responseSchema === "case") return "light";
  return "none";
}

/** Whether the ask mentions external/current info that heuristically maps to `research`. */
export function mentionsExternalInfo(text: string, now: () => Date): boolean {
  if (text.length === 0) return false;
  if (RESEARCH_CUES.test(text)) return true;
  if (URL_CUE.test(text)) return true;
  if (WHO_WHAT_IS.test(text)) return true;
  const yearMatch = text.match(/\b(20\d{2})\b/);
  if (yearMatch?.[1] && Number(yearMatch[1]) >= now().getFullYear()) return true;
  return false;
}

function schemaFor(mode: BlueyMode, task: AITask): ResponseSchemaId {
  // Modes with generic schemas get upgraded when the ask is clearly technical.
  const generic = mode.responseSchema === "answer" || mode.responseSchema === "suggested-response";
  if (task === "coding" && generic) return "coding";
  if (task === "system_design" && generic) return "system-design";
  return mode.responseSchema;
}

function responseTypeFor(schemaId: ResponseSchemaId, task: AITask): ResponseType {
  switch (schemaId) {
    case "coding":
      return "code";
    case "system-design":
      return "system-design";
    case "suggested-response":
    case "behavioral":
    case "case":
    case "sales":
    case "recruiting":
      return "suggestion";
    case "meeting":
      return task === "summarization" ? "summary" : "answer";
    case "lecture":
      return task === "summarization" ? "summary" : "answer";
    case "answer":
    default:
      if (task === "research") return "research";
      if (task === "summarization") return "summary";
      return "answer";
  }
}

function averageOcrConfidence(snapshot: ContextSnapshot): number {
  const blocks = snapshot.ocr?.blocks ?? [];
  if (blocks.length === 0) return snapshot.ocr?.text ? 1 : 0;
  return blocks.reduce((sum, block) => sum + block.confidence, 0) / blocks.length;
}

export const VISION_TEXT_SUFFICIENCY_CHARS = 200;

/** Classify the current ask into task, vision need, reasoning depth, schema and answer shape. */
export function classifyIntent(input: IntentInput): Intent {
  const { snapshot, mode, detectedEvent, trigger } = input;
  const now = input.now ?? (() => new Date());

  const question = currentQuestionText(snapshot, {
    instruction: input.instruction,
    detectedEvent,
  });
  const ocrText = snapshot.ocr?.text ?? "";
  const axText = snapshot.accessibility?.visibleText ?? "";

  // ── Task ────────────────────────────────────────────────────────────────
  let task: AITask = defaultTaskFor(mode);
  const codingAsked = CODING_CUES.test(question) || detectedEvent?.type === "coding_problem";
  const codingVisible = CODING_SCREEN_MARKERS.test(ocrText) && ocrText.length > 80;
  const designAsked = SYSTEM_DESIGN_CUES.test(question) || mode.responseSchema === "system-design";
  // A multiple-choice or compare-two-responses question about code is still
  // an assessment question: the screen's code markers alone must not turn it
  // into a "solve this problem" coding task.
  const assessment = detectAssessmentShape(question, ocrText);
  const codingFromScreen = codingVisible && assessment === null;
  // A question heard in the live conversation is answered now, as speech; it
  // never goes out to the web ("my current role" is not a research cue).
  const spokenTrigger = trigger === "shortcut_generate" || trigger === "detected_event";

  if (SUMMARIZE_CUES.test(question)) {
    task = "summarization";
  } else if (designAsked) {
    task = "system_design";
  } else if (codingAsked || codingFromScreen || mode.responseSchema === "coding") {
    task = "coding";
  } else if (!spokenTrigger && mentionsExternalInfo(question, now)) {
    task = "research";
  } else {
    task = "answer";
  }

  // ── Vision ──────────────────────────────────────────────────────────────
  const hasScreen = Boolean(snapshot.screen?.image ?? snapshot.screen?.frameId);
  const textAvailable = ocrText.length + axText.length;
  const insufficientText = textAvailable < VISION_TEXT_SUFFICIENCY_CHARS;
  const refersToVisual = VISUAL_REFERENCE_CUES.test(question);
  const codingLowOcr = task === "coding" && averageOcrConfidence(snapshot) < 0.55;
  const visionRequired = hasScreen && (insufficientText || refersToVisual || codingLowOcr);

  // ── Depth ───────────────────────────────────────────────────────────────
  const reasoning = reasoningFor(task, mode);
  const latency = slowerOf(mode.preferredLatency, minimumLatencyFor(task));

  const schemaId = schemaFor(mode, task);
  const responseType = responseTypeFor(schemaId, task);
  const answerShape = detectAnswerShape({ question, screenText: ocrText, task, schemaId, trigger });

  return { task, visionRequired, reasoning, latency, responseType, schemaId, answerShape };
}
