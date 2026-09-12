/** Build the `AIRequest` handed to the Rust model router. */

import type { Intent } from "@/context/relevance";
import type { AIMessage, AIRequest, AITask, AnswerShape, ResponseLength, Session } from "@/lib/types";

export interface BuildAIRequestArgs {
  requestId: string;
  generation: number;
  intent: Intent;
  messages: AIMessage[];
  contextTokens: number;
  responseLength: ResponseLength;
  session?: Session | null;
  useStructuredOutput?: boolean;
  outputSchema?: AIRequest["outputSchema"];
  now: () => Date;
}

const LENGTH_TOKENS: Record<ResponseLength, number> = {
  concise: 600,
  balanced: 1200,
  detailed: 2400,
};

const TASK_FLOOR_TOKENS: Partial<Record<AITask, number>> = {
  coding: 1600,
  system_design: 2400,
  summarization: 1600,
  research: 1200,
};

/**
 * Per-shape floors: a pick or a yes/no needs little, a spoken answer or a
 * piece of writing must never be cut mid-sentence, code and designs need
 * room for the artefact itself.
 */
const SHAPE_FLOOR_TOKENS: Record<AnswerShape, number> = {
  choice: 400,
  boolean: 400,
  fill_in: 400,
  calculation: 400,
  short_answer: 400,
  compare: 600,
  explain: 600,
  spoken: 700,
  written: 700,
  code: 2000,
  design: 3000,
  summary: 1600,
};

/** The JSON envelope, field names, section titles and string escaping cost tokens on top of the prose. */
export const STRUCTURED_OUTPUT_OVERHEAD_TOKENS = 200;

/**
 * Output budget: the style's length, raised to the task's and the shape's
 * floors, plus the structured-output overhead when a schema is sent. Never
 * below the pre-shape values (`length` and `task` alone).
 */
export function maxOutputTokensFor(
  length: ResponseLength,
  task: AITask,
  shape?: AnswerShape,
  structured = false,
): number {
  const floor = Math.max(TASK_FLOOR_TOKENS[task] ?? 0, shape ? SHAPE_FLOOR_TOKENS[shape] : 0);
  const base = Math.max(LENGTH_TOKENS[length], floor);
  return structured ? base + STRUCTURED_OUTPUT_OVERHEAD_TOKENS : base;
}

function temperatureFor(task: AITask): number {
  switch (task) {
    case "coding":
      return 0.2;
    case "classification":
      return 0;
    case "system_design":
    case "summarization":
      return 0.4;
    default:
      return 0.6;
  }
}

export function buildAIRequest(args: BuildAIRequestArgs): AIRequest {
  const { requestId, generation, intent, messages, contextTokens, responseLength, session, now } = args;
  const structured = Boolean(args.outputSchema) && args.useStructuredOutput !== false;

  const request: AIRequest = {
    requestId,
    generation,
    task: intent.task,
    latencyBudget: intent.latency,
    reasoning: intent.reasoning,
    visionRequired: intent.visionRequired,
    contextTokens,
    messages,
    maxOutputTokens: maxOutputTokensFor(responseLength, intent.task, intent.answerShape, structured),
    temperature: temperatureFor(intent.task),
    createdAt: now().toISOString(),
  };
  if (session) request.sessionId = session.id;
  if (structured && args.outputSchema) {
    request.outputSchema = args.outputSchema;
  }
  return request;
}
