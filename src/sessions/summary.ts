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
import { estimateTokens } from "@/context/fusion";
import type { SummarizeInput } from "@/lib/engine-contract";
import type { AIMessage, AIRequest, JsonSchemaSpec, SessionSummary, TranscriptSegment } from "@/lib/types";
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

// Strict-mode providers answer optional fields with `null` (see `src/modes/schemas.ts`).
const tolerantSummary = z.object({
  overview: z.string().nullish(),
  topics: z.array(z.string()).nullish(),
  questions: z.array(z.string()).nullish(),
  answers: z.array(z.string()).nullish(),
  decisions: z.array(z.string()).nullish(),
  actionItems: z.array(z.string()).nullish(),
  openItems: z.array(z.string()).nullish(),
  improvements: z.array(z.string()).nullish(),
  sections: z.array(z.object({ title: z.string(), content: z.string() })).nullish(),
});

const MAX_TRANSCRIPT_TOKENS = 6000;
/** Share of the transcript budget spent on the start of a long session; the rest keeps its end. */
const HEAD_SHARE = 1 / 3;
/** Room kept for the "[… N lines … omitted …]" line. */
const OMISSION_MARKER_TOKENS = 40;
const MAX_RESPONSE_CHARS = 400;

interface TranscriptExcerpt {
  text: string;
  /** Set when the middle of a long session did not fit: what the user is told. */
  omittedNote?: string;
}

function mmss(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  return `${String(Math.floor(total / 60)).padStart(2, "0")}:${String(total % 60).padStart(2, "0")}`;
}

/**
 * The transcript within the budget. A long session keeps its start (context, agenda) and its
 * end (decisions, next steps) and says what was left out — to the model in the text and to the
 * user in the summary (AI-007).
 */
export function transcriptExcerpt(transcript: TranscriptSegment[]): TranscriptExcerpt | null {
  const segments = transcript.filter((s) => s.finalized && s.text.trim().length > 0);
  if (segments.length === 0) return null;
  const lines = segments.map(
    (s) => `${s.speaker ?? (s.source === "microphone" ? "You" : "Speaker")}: ${s.text}`,
  );
  const text = lines.join("\n");
  if (estimateTokens(text) <= MAX_TRANSCRIPT_TOKENS) return { text };

  const costs = lines.map((line) => estimateTokens(line) + 1);
  let head = 0;
  let used = 0;
  const headBudget = MAX_TRANSCRIPT_TOKENS * HEAD_SHARE;
  while (head < lines.length && used + costs[head]! <= headBudget) {
    used += costs[head]!;
    head += 1;
  }
  let tail = lines.length;
  const budget = MAX_TRANSCRIPT_TOKENS - OMISSION_MARKER_TOKENS;
  while (tail > head && used + costs[tail - 1]! <= budget) {
    tail -= 1;
    used += costs[tail]!;
  }
  if (head === 0 && tail === lines.length) return { text: compressKeepTail(text, MAX_TRANSCRIPT_TOKENS) };

  const from = segments[head]!.startTime;
  const to = segments[tail - 1]!.endTime;
  const range = `${mmss(from)}–${mmss(to)}`;
  const minutes = Math.max(1, Math.round((to - from) / 60_000));
  return {
    text: [
      ...lines.slice(0, head),
      `[… ${tail - head} lines from the middle of the session (${range}) omitted …]`,
      ...lines.slice(tail),
    ].join("\n"),
    omittedNote: `This session was long: the summary covers its beginning and end; about ${minutes} min in the middle (${range}) was not included.`,
  };
}

/** Compile the session material into one user-message body. */
export function renderSummaryInput(input: SummarizeInput): string {
  const parts: string[] = [];

  const excerpt = transcriptExcerpt(input.transcript);
  if (excerpt) parts.push(`### Transcript\n${excerpt.text}`);

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
  const material = renderSummaryInput(input);
  const messages: AIMessage[] = [
    { role: "system", content: [{ type: "text", text: SUMMARY_SYSTEM }] },
    {
      role: "user",
      content: [{ type: "text", text: `${material}\n\n${summaryTaskFor(input.mode)}` }],
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
    contextTokens: Math.ceil(material.length / 4),
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
    throw (
      outcome.error ?? {
        kind: "ai" as const,
        code: "ai.summary_failed",
        message: "Session summary generation failed",
        recoverable: true,
      }
    );
  }

  const parsed = parseSummaryOutput(outcome.text);
  const omittedNote = transcriptExcerpt(input.transcript)?.omittedNote;
  const overview = parsed.overview ?? "";
  return {
    id: `sum_${resolved.idGen()}`,
    sessionId: input.session.id,
    modeId: input.mode.id,
    // Stored with the summary, so the detail view and the export say it too.
    overview: omittedNote ? `${overview}\n\n${omittedNote}`.trim() : overview,
    topics: parsed.topics ?? [],
    questions: parsed.questions ?? [],
    answers: parsed.answers ?? [],
    decisions: parsed.decisions ?? [],
    actionItems: parsed.actionItems ?? [],
    openItems: parsed.openItems ?? [],
    improvements: parsed.improvements ?? [],
    sections: parsed.sections ?? undefined,
    createdAt: resolved.now().toISOString(),
  };
}
