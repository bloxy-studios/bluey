/**
 * Post-session summary: builds a summarization request (mode-structured
 * schema), streams it, and parses the result into a `SessionSummary`.
 * Tolerant: a malformed model response degrades to an overview-only summary
 * rather than failing.
 */

import * as z from "zod";
import { SUMMARY_SYSTEM, summaryTaskFor } from "@/ai/prompts";
import { streamRequest, type StreamApi } from "@/ai/stream";
import { compressKeepTail } from "@/context/budget";
import type { SummarizeInput } from "@/lib/engine-contract";
import type { AIMessage, AIRequest, JsonSchemaSpec, SessionSummary } from "@/lib/types";
import { parseJsonLoose } from "@/modes/schemas";

const summarySchema = z.object({
  overview: z.string(),
  topics: z.array(z.string()),
  questions: z.array(z.string()),
  answers: z.array(z.string()),
  decisions: z.array(z.string()),
  actionItems: z.array(z.string()),
  openItems: z.array(z.string()),
  improvements: z.array(z.string()),
  sections: z.array(z.object({ title: z.string(), content: z.string() })).optional(),
});

export function summaryOutputSchema(): JsonSchemaSpec {
  return {
    name: "bluey_session_summary",
    schema: z.toJSONSchema(summarySchema) as Record<string, unknown>,
    strict: true,
  };
}

const tolerantSummary = z.object({
  overview: z.string().optional(),
  topics: z.array(z.string()).optional(),
  questions: z.array(z.string()).optional(),
  answers: z.array(z.string()).optional(),
  decisions: z.array(z.string()).optional(),
  actionItems: z.array(z.string()).optional(),
  openItems: z.array(z.string()).optional(),
  improvements: z.array(z.string()).optional(),
  sections: z.array(z.object({ title: z.string(), content: z.string() })).optional(),
});

const MAX_TRANSCRIPT_TOKENS = 6000;
const MAX_RESPONSE_CHARS = 400;

/** Compile the session material into one user-message body. */
export function renderSummaryInput(input: SummarizeInput): string {
  const parts: string[] = [];

  const transcriptLines = input.transcript
    .filter((s) => s.finalized && s.text.trim().length > 0)
    .map((s) => `${s.speaker ?? (s.source === "microphone" ? "You" : "Speaker")}: ${s.text}`)
    .join("\n");
  if (transcriptLines.length > 0) {
    parts.push(`### Transcript\n${compressKeepTail(transcriptLines, MAX_TRANSCRIPT_TOKENS)}`);
  }

  if (input.responses.length > 0) {
    const lines = input.responses.map((r) => {
      const body = r.content.length > MAX_RESPONSE_CHARS ? `${r.content.slice(0, MAX_RESPONSE_CHARS)}…` : r.content;
      return `- ${r.title ?? r.type}: ${body}`;
    });
    parts.push(`### Responses given\n${lines.join("\n")}`);
  }

  if (input.events.length > 0) {
    const lines = input.events.map((e) => `- ${e.title}${e.detail ? `: ${e.detail}` : ""}`);
    parts.push(`### Timeline events\n${lines.join("\n")}`);
  }

  if (input.notes.length > 0) {
    parts.push(`### User notes\n${input.notes.map((n) => `- ${n.content}`).join("\n")}`);
  }

  return parts.join("\n\n");
}

export interface SummaryDeps {
  api: StreamApi;
  now?: () => Date;
  idGen?: () => string;
}

function buildSummaryRequest(input: SummarizeInput, deps: Required<SummaryDeps>): AIRequest {
  const messages: AIMessage[] = [
    { role: "system", content: [{ type: "text", text: SUMMARY_SYSTEM }] },
    {
      role: "user",
      content: [
        { type: "text", text: `${renderSummaryInput(input)}\n\n${summaryTaskFor(input.mode)}` },
      ],
    },
  ];
  return {
    requestId: `req_${deps.idGen()}`,
    sessionId: input.session.id,
    generation: 1,
    task: "summarization",
    latencyBudget: "balanced",
    reasoning: "light",
    visionRequired: false,
    contextTokens: Math.ceil(renderSummaryInput(input).length / 4),
    messages,
    outputSchema: summaryOutputSchema(),
    maxOutputTokens: 2000,
    temperature: 0.4,
    createdAt: deps.now().toISOString(),
  };
}

/** Parse the raw model text into summary fields (tolerant, never throws). */
export function parseSummaryOutput(text: string): z.infer<typeof tolerantSummary> {
  const json = parseJsonLoose(text);
  if (json !== null && typeof json === "object") {
    const parsed = tolerantSummary.safeParse(json);
    if (parsed.success) return parsed.data;
  }
  return { overview: text.trim() };
}

/** Generate (but do not persist) a session summary. */
export async function generateSessionSummary(
  input: SummarizeInput,
  deps: SummaryDeps,
): Promise<SessionSummary> {
  const resolved: Required<SummaryDeps> = {
    api: deps.api,
    now: deps.now ?? (() => new Date()),
    idGen: deps.idGen ?? (() => crypto.randomUUID()),
  };
  const request = buildSummaryRequest(input, resolved);
  const outcome = await streamRequest(request, {}, resolved.api).done;

  if (outcome.finishReason === "error") {
    throw outcome.error ?? {
      kind: "ai" as const,
      code: "ai.summary_failed",
      message: "Session summary generation failed",
      recoverable: true,
    };
  }

  const parsed = parseSummaryOutput(outcome.text);
  return {
    id: `sum_${resolved.idGen()}`,
    sessionId: input.session.id,
    modeId: input.mode.id,
    overview: parsed.overview ?? "",
    topics: parsed.topics ?? [],
    questions: parsed.questions ?? [],
    answers: parsed.answers ?? [],
    decisions: parsed.decisions ?? [],
    actionItems: parsed.actionItems ?? [],
    openItems: parsed.openItems ?? [],
    improvements: parsed.improvements ?? [],
    sections: parsed.sections,
    createdAt: resolved.now().toISOString(),
  };
}
