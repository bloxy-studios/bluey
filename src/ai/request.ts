/** Build the `AIRequest` handed to the Rust model router. */

import type { Intent } from "@/context/relevance";
import type { AIMessage, AIRequest, AITask, ResponseLength, Session } from "@/lib/types";

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

export function maxOutputTokensFor(length: ResponseLength, task: AITask): number {
  const base = LENGTH_TOKENS[length];
  const floor = TASK_FLOOR_TOKENS[task] ?? 0;
  return Math.max(base, floor);
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

  const request: AIRequest = {
    requestId,
    generation,
    task: intent.task,
    latencyBudget: intent.latency,
    reasoning: intent.reasoning,
    visionRequired: intent.visionRequired,
    contextTokens,
    messages,
    maxOutputTokens: maxOutputTokensFor(responseLength, intent.task),
    temperature: temperatureFor(intent.task),
    createdAt: now().toISOString(),
  };
  if (session) request.sessionId = session.id;
  if (args.outputSchema && args.useStructuredOutput !== false) {
    request.outputSchema = args.outputSchema;
  }
  return request;
}
