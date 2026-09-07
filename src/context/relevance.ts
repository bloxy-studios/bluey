/**
 * Intent classification for the current ask: which AI task is this, does it
 * need vision, how deep should the model think, and which structured schema
 * should the response use.
 */

import type {
  AITask,
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

/** Classify the current ask into task, vision need, reasoning depth and schema. */
export function classifyIntent(input: IntentInput): Intent {
  const { snapshot, mode, detectedEvent } = input;
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

  if (SUMMARIZE_CUES.test(question)) {
    task = "summarization";
  } else if (designAsked) {
    task = "system_design";
  } else if (codingAsked || codingVisible || mode.responseSchema === "coding") {
    task = "coding";
  } else if (mentionsExternalInfo(question, now)) {
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

  return { task, visionRequired, reasoning, latency, responseType, schemaId };
}
