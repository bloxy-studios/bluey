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
  AnswerVoice,
  BlueyMode,
  ContextSnapshot,
  DetectedEvent,
  LatencyBudget,
  ReasoningLevel,
  ResponseSchemaId,
  ResponseType,
} from "@/lib/types";
import { defaultTaskFor } from "@/modes/registry";
import { BEHAVIORAL_MARKERS } from "@/transcript/classifier";
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
  voice: AnswerVoice;
}

const CODING_CUES =
  /\b(implement|write (a|the) (function|method|program|algorithm)|leetcode|time complexity|space complexity|big[- ]o|debug|fix (this|the) (bug|code|test)|refactor|unit test|regex|algorithm)\b/i;

/** A problem to solve: a judge's statement or verdict. Upgrades any ask to coding. */
const PROBLEM_SCREEN_MARKERS = /(Example \d+:|Constraints:|Input:[\s\S]{0,400}Output:|Time Limit Exceeded|Wrong Answer)/;
/** Source code in an editor. Upgrades to coding only when the ask is to solve, fix or write it. */
const SOURCE_SCREEN_MARKERS =
  /(\bfunction\s+\w+\s*\(|\bdef\s+\w+\s*\(|\bclass\s+[A-Z]\w*\s*(\(|\{|:|extends\b|implements\b)|```)/;
/** A runtime or compiler error, a failing test or a stack trace. */
const ERROR_SCREEN_MARKERS =
  /(Traceback \(most recent call last\)|panicked at|error\[E\d{4}\]|\b[A-Z]\w*(Error|Exception)\b|Segmentation fault|\bFAILED\b|Uncaught )/;
/** "why … fail/error/bug", "what's wrong with", "debug", "fix the bug". */
const DEBUG_CUES =
  /\b(debug|fix (this|the|my) (bug|error|test|crash)|what'?s wrong with|why\b[^?\n]{0,60}\b(fail\w*|error\w*|bug\w*|crash\w*|throw\w*|broken|wrong|not work\w*|doesn'?t work))/i;

const SYSTEM_DESIGN_CUES =
  /\b(design (a|an|the) (system|url shortener|rate limiter|feed|chat|notification|search|cache|scheduler|queue)|system design|high[- ]level (architecture|design)|scal(e|ability|ing)|shard|load balanc|distributed|architecture for|design .{0,40}\b(like|similar to)\b)\b/i;

const SUMMARIZE_CUES = /\b(summari[sz]e|recap|sum up|tl;?dr|key takeaways|meeting notes|wrap[- ]up)\b/i;

const RESEARCH_CUES = /\b(research|look up|latest|news|recent(ly)?|current(ly)?|deep dive|compare vendors?|competitors? of)\b/i;
const WHO_WHAT_IS = /\b(who|what)\s+(is|are|was|were)\s+([A-Z][\w.&-]*)/;
const URL_CUE = /https?:\/\/\S+/i;

const VISUAL_REFERENCE_CUES =
  /\b(this|that|the)\s+(chart|graph|diagram|screenshot|image|picture|figure|design|mockup|slide|table|plot)\b|\bon (my|the) screen\b|\bwhat am i looking at\b/i;

/** Words a chart, diagram or figure prints on screen (titles, legends, axes). */
const VISUAL_SCREEN_CUES =
  /\b(chart|graph|diagram|figure|fig\.|legend|axis|plot|histogram|scatter|flowchart|heat ?map|dashboard)\b|\bQ1\b[\s\S]{0,40}\bQ2\b/i;
/** A line that is only a number, a percentage or an amount — an axis tick or a data label. */
const NUMERIC_LABEL_LINE = /^\s*[$€£]?-?\d[\d.,]*\s*(%|[kKmMbB])?\s*$/;
const MIN_NUMERIC_LABELS = 4;

/** Whether the OCR reads like a chart or figure: visual words, or a run of bare axis numbers. */
function screenLooksVisual(ocrText: string): boolean {
  if (VISUAL_SCREEN_CUES.test(ocrText)) return true;
  const labels = ocrText.split("\n").filter((line) => NUMERIC_LABEL_LINE.test(line)).length;
  return labels >= MIN_NUMERIC_LABELS;
}

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
/** "A or B?" is a pick, not a yes/no ("… or not" still is one) (AI-005). */
const EITHER_OR = /\bor\b(?!\s+not\b)/i;
/** A prediction has no yes/no answer yet: a best estimate and what it hinges on. */
const FORECAST_CUES =
  /\b(predict|forecast|likely|likelihood|odds|chances?)\b|\bwill\b.*\b(next|tomorrow|soon|by 20\d\d|in 20\d\d|this (week|month|quarter|year))\b/i;

const CALCULATION_CUES =
  /\b(calculate|compute|how (many|much)|what is the (total|sum|difference|product|average|mean|median|mode|probability|percentage|percent|value|result|remainder|area|volume|distance|speed|rate)|solve for|evaluate the expression|round(ed)? to|to the nearest)\b|\d\s*[+\-*/×÷^=%]\s*\d/i;

const WRITTEN_CUES =
  /\b(write|draft|compose|rewrite|reword|rephrase|reply to|respond to)\b[^.?\n]{0,40}\b(email|e-mail|message|reply|comment|post|note|letter|essay|paragraph|statement|bio|text|dm|tweet|summary)\b|\b(cover letter|essay|email reply)\b/i;

const EXPLAIN_CUES =
  /\b(why|how (do|does|did|would|should|can|could|is|are|to|about)|explain|describe|walk (me )?through|what happens|difference between|what does .{1,40} mean|elaborate|in detail|tell me about)\b/i;
/** An explanation the user wants for themselves ("so I understand", "what does … mean", "why …"). */
const EXPLAIN_TO_ME_CUES =
  /\b(explain|so (that )?I (can )?understand|help me understand|what does .{1,40} mean|what is meant by|in simple terms|eli5|why)\b/i;
/** A question put to the user ("why do you…", "tell me about your…") — words to say, not an explanation. */
const ADDRESSED_TO_USER = /\b(you|your|yourself)\b/i;
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
  if (YES_NO_OPENER.test(q) && q.length <= 160 && !EXPLAIN_CUES.test(q) && !EITHER_OR.test(q) && !FORECAST_CUES.test(q)) {
    return "boolean";
  }
  if (CALCULATION_CUES.test(q) && /\d/.test(both)) return "calculation";
  return null;
}

export interface AnswerShapeInput {
  question: string;
  screenText: string;
  task: AITask;
  schemaId: ResponseSchemaId;
  trigger?: AskTrigger;
  /** A bug to fix (see `classifyIntent`): the debug shape beats the task's `code`. */
  debugging?: boolean;
}

/** The shape the answer should take (see the detection order above). */
export function detectAnswerShape(input: AnswerShapeInput): AnswerShape {
  const { screenText, task, schemaId, trigger } = input;
  const q = input.question.trim();
  const spokenTrigger = trigger === "shortcut_generate" || trigger === "detected_event";
  // A typed request for an explanation in a conversational mode is for me to
  // read, not words to say (MODE-002); a question put to me stays spoken.
  const explainToMe = !spokenTrigger && EXPLAIN_TO_ME_CUES.test(q) && !ADDRESSED_TO_USER.test(q);
  const spoken = spokenTrigger || (SPOKEN_SCHEMAS.has(schemaId) && !explainToMe);
  const assessment = detectAssessmentShape(q, screenText, { spoken });
  if (assessment) return assessment;
  if (input.debugging && !spoken) return "debug";
  if (task === "coding") return "code";
  if (task === "system_design") return "design";
  if (task === "summarization") return "summary";
  if (WRITTEN_CUES.test(q)) return "written";
  if (spoken) return "spoken";
  if (EXPLAIN_CUES.test(q)) return "explain";
  if (q.length > 0 && q.length <= 120 && SHORT_ANSWER_OPENER.test(q)) return "short_answer";
  // An either/or or a forecast phrased as yes/no: the pick or the best estimate (AI-005).
  if (YES_NO_OPENER.test(q) && q.length <= 160 && (EITHER_OR.test(q) || FORECAST_CUES.test(q))) return "short_answer";
  return "explain";
}

const SUBMITTED_SHAPES: ReadonlySet<AnswerShape> = new Set<AnswerShape>([
  "choice",
  "boolean",
  "fill_in",
  "calculation",
  "compare",
  "short_answer",
  "written",
  "code",
  "design",
]);

/**
 * Whose words the answer is: speech for spoken shapes and heard questions,
 * what I submit for picks, values, texts and solutions, and an explanation
 * addressed to me for explain, summary and debug.
 */
export function voiceFor(shape: AnswerShape, trigger?: AskTrigger): AnswerVoice {
  if (shape === "spoken" || trigger === "shortcut_generate" || trigger === "detected_event") return "speak-as-user";
  return SUBMITTED_SHAPES.has(shape) ? "write-as-user" : "explain-to-user";
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

interface SchemaInput {
  debugging: boolean;
  spokenTrigger: boolean;
  behavioral: boolean;
}

function schemaFor(mode: BlueyMode, task: AITask, input: SchemaInput): ResponseSchemaId {
  // Modes with generic schemas get upgraded when the ask is clearly technical.
  const generic = mode.responseSchema === "answer" || mode.responseSchema === "suggested-response";
  // A fix is an answer, not a full solution with complexity and edge cases.
  if (input.debugging && (generic || mode.responseSchema === "coding")) return "answer";
  // A technical mode asked something non-technical: speech when heard, else a plain answer.
  const technical = mode.responseSchema === "coding" || mode.responseSchema === "system-design";
  if (technical && task !== "coding" && task !== "system_design") {
    if (!input.spokenTrigger) return "answer";
    return input.behavioral ? "behavioral" : "suggested-response";
  }
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

/** Upper bound on the screen text the classifier's regexes scan. */
const SCREEN_TEXT_CAP = 6000;

/**
 * Everything readable on screen: OCR plus the accessibility text, the
 * selection and the focused value. Rust drops OCR lines that repeat an AX
 * line, so OCR alone can miss the very question or problem (CTX-011).
 */
function screenTextOf(snapshot: ContextSnapshot): string {
  const ax = snapshot.accessibility;
  return [snapshot.ocr?.text, ax?.visibleText, ax?.selectedText, ax?.focusedElement?.value]
    .filter((text): text is string => typeof text === "string" && text.trim().length > 0)
    .join("\n")
    .slice(0, SCREEN_TEXT_CAP);
}

/** Classify the current ask into task, vision need, reasoning depth, schema and answer shape. */
export function classifyIntent(input: IntentInput): Intent {
  const { snapshot, mode, detectedEvent, trigger } = input;
  const now = input.now ?? (() => new Date());

  // On ⌘↵ and assist the screen is the subject: with nothing typed, a
  // question heard minutes ago ("Can you see my screen?") must not become the
  // question and decide the task or shape (CTX-001).
  const screenTrigger = trigger === "shortcut_capture" || trigger === "assist";
  const question =
    screenTrigger && !detectedEvent
      ? (input.instruction?.trim() ?? "")
      : currentQuestionText(snapshot, { instruction: input.instruction, detectedEvent });
  const ocrText = snapshot.ocr?.text ?? "";
  const screenText = screenTextOf(snapshot);

  // ── Task ────────────────────────────────────────────────────────────────
  let task: AITask = defaultTaskFor(mode);
  const codingAsked = CODING_CUES.test(question) || detectedEvent?.type === "coding_problem";
  const problemVisible = PROBLEM_SCREEN_MARKERS.test(screenText) && screenText.length > 80;
  const sourceVisible = SOURCE_SCREEN_MARKERS.test(screenText);
  // Code in an editor is the subject of a solve, fix or write request — not
  // of "what does this do?" or "who wrote this?" (MODE-001).
  const asksToSolve =
    question.length === 0 || codingAsked || !(EXPLAIN_CUES.test(question) || SHORT_ANSWER_OPENER.test(question));
  const codingVisible = problemVisible || (sourceVisible && asksToSolve);
  const designCue =
    SYSTEM_DESIGN_CUES.test(question) || (screenTrigger && SYSTEM_DESIGN_CUES.test(screenText));
  // A coding or design mode forces its task only for a technical ask — a cue,
  // or ⌘↵/assist over code. "Tell me about yourself" in a coding interview is
  // answered as speech, not as code (MODE-004).
  const technicalAsk = codingAsked || designCue || (screenTrigger && (problemVisible || sourceVisible));
  const designAsked = designCue || (mode.responseSchema === "system-design" && technicalAsk);
  // A multiple-choice or compare-two-responses question about code is still
  // an assessment question: the screen's code markers alone must not turn it
  // into a "solve this problem" coding task.
  const assessment = detectAssessmentShape(question, screenText);
  const codingFromScreen = codingVisible && assessment === null;
  // A question heard in the live conversation is answered now, as speech; it
  // never goes out to the web ("my current role" is not a research cue).
  const spokenTrigger = trigger === "shortcut_generate" || trigger === "detected_event";
  // A bug to fix — an error beside code, or "why is this failing" — wants the
  // fix first and only the changed lines, not a fresh full solution.
  const debugging =
    !spokenTrigger &&
    !problemVisible &&
    assessment === null &&
    (sourceVisible || codingAsked) &&
    (ERROR_SCREEN_MARKERS.test(screenText) || DEBUG_CUES.test(question));

  if (SUMMARIZE_CUES.test(question)) {
    task = "summarization";
  } else if (designAsked) {
    task = "system_design";
  } else if (codingAsked || codingFromScreen || debugging || (mode.responseSchema === "coding" && technicalAsk)) {
    task = "coding";
  } else if (!spokenTrigger && mentionsExternalInfo(question, now)) {
    task = "research";
  } else {
    task = "answer";
  }

  // ── Vision ──────────────────────────────────────────────────────────────
  // Sufficiency is measured on OCR only: accessibility text is mostly app
  // chrome and says nothing about whether the pixels carry the content. When
  // the screen is the subject (⌘↵, assist), a chart or figure on it needs
  // the image however much label text OCR read off it (CTX-009).
  const hasScreen = Boolean(snapshot.screen?.image ?? snapshot.screen?.frameId);
  const insufficientText = ocrText.length < VISION_TEXT_SUFFICIENCY_CHARS;
  const screenIsSubject = question.length === 0 || screenTrigger;
  const refersToVisual =
    VISUAL_REFERENCE_CUES.test(question) || (screenIsSubject && screenLooksVisual(ocrText));
  const codingLowOcr = task === "coding" && averageOcrConfidence(snapshot) < 0.55;
  const visionRequired = hasScreen && (insufficientText || refersToVisual || codingLowOcr);

  // ── Depth ───────────────────────────────────────────────────────────────
  const reasoning = reasoningFor(task, mode);
  const latency = slowerOf(mode.preferredLatency, minimumLatencyFor(task));

  const behavioral = detectedEvent?.type === "behavioral_question" || BEHAVIORAL_MARKERS.test(question);
  const schemaId = schemaFor(mode, task, { debugging, spokenTrigger, behavioral });
  const responseType = responseTypeFor(schemaId, task);
  const answerShape = detectAnswerShape({ question, screenText, task, schemaId, trigger, debugging });

  const voice = voiceFor(answerShape, trigger);

  return { task, visionRequired, reasoning, latency, responseType, schemaId, answerShape, voice };
}
