/**
 * stdio JSON Lines protocol between the Rust backend and this sidecar.
 * Mirrors `docs/AGENT_SIDECAR_PROTOCOL.md`.
 *
 * The sidecar is a separate program: the shared wire types are COPIED here
 * (from `src/lib/types/ai.ts` / `src/lib/types/response.ts`), never imported
 * from the app source tree.
 */

import { z } from "zod";

// ── Wire envelopes ───────────────────────────────────────────────────────────

export type RequestId = string | number;

export interface RpcRequest {
  id: RequestId;
  method: string;
  params?: unknown;
}

/** Error payload used in both `{id,error}` responses and `research.failed`. */
export interface WireError {
  code: string;
  message: string;
  kind: string;
}

// ── Deep research request (mirrors DeepResearchRequest in src/lib/types/ai.ts) ──

export const RESEARCH_TOOL_NAMES = ["exa_search", "firecrawl_scrape", "document_read"] as const;
export type ResearchToolName = (typeof RESEARCH_TOOL_NAMES)[number];

export interface DeepResearchRequest {
  jobId: string;
  sessionId?: string;
  /** PUBLIC query only — never include private context. */
  query: string;
  goal: string;
  maxTurns?: number;
  tools: ResearchToolName[];
  /** Document ids the agent may read via the document_read tool. */
  allowedDocumentIds?: string[];
  /** Optional model override (protocol doc allows it on the wire). */
  model?: string;
}

export const deepResearchRequestSchema = z.object({
  jobId: z.string().min(1),
  sessionId: z.string().optional(),
  query: z.string().min(1),
  goal: z.string().min(1),
  maxTurns: z.number().int().positive().optional(),
  tools: z.array(z.enum(RESEARCH_TOOL_NAMES)).min(1),
  allowedDocumentIds: z.array(z.string()).optional(),
  model: z.string().min(1).optional(),
});

// ── Citations (mirrors Citation in src/lib/types/response.ts, sans local id) ──

export interface WireCitation {
  title: string;
  url: string;
  snippet?: string;
}

// ── Events (agent → Rust) ────────────────────────────────────────────────────

export interface ResearchStartedData {
  jobId: string;
  model: string;
}

export interface ResearchProgressData {
  jobId: string;
  message: string;
}

export interface ResearchToolCallData {
  jobId: string;
  tool: string;
  input: Record<string, unknown>;
}

export interface ResearchTextDeltaData {
  jobId: string;
  text: string;
}

export interface ResearchCompletedData {
  jobId: string;
  /** Markdown report. */
  report: string;
  citations: WireCitation[];
  turns: number;
  totalMs: number;
  usage: { inputTokens: number; outputTokens: number };
}

export interface ResearchFailedData {
  jobId: string;
  error: WireError;
}

export interface DocumentRequestData {
  requestId: string;
  documentId: string;
}

export interface SidecarEventMap {
  "research.started": ResearchStartedData;
  "research.progress": ResearchProgressData;
  "research.toolCall": ResearchToolCallData;
  "research.textDelta": ResearchTextDeltaData;
  "research.completed": ResearchCompletedData;
  "research.failed": ResearchFailedData;
  "document.request": DocumentRequestData;
}

export type SidecarEventName = keyof SidecarEventMap;

// ── document.response (Rust → agent; answers a document.request event) ──────

export const documentResponseSchema = z.object({
  requestId: z.string().min(1),
  documentId: z.string().min(1),
  text: z.string().optional(),
  error: z.string().optional(),
});

export type DocumentResponseParams = z.infer<typeof documentResponseSchema>;

// ── Line parsing ─────────────────────────────────────────────────────────────

export type ParsedLine =
  | { ok: true; request: RpcRequest }
  | { ok: false; error: string; id: RequestId | null };

/** Parse one JSON-Lines request. Never throws. */
export function parseRequestLine(line: string): ParsedLine {
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch {
    return { ok: false, error: "invalid JSON", id: null };
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return { ok: false, error: "request must be a JSON object", id: null };
  }
  const obj = value as Record<string, unknown>;
  const id = obj["id"];
  const idOk = typeof id === "string" || typeof id === "number";
  if (typeof obj["method"] !== "string" || obj["method"].length === 0) {
    return { ok: false, error: "missing method", id: idOk ? (id as RequestId) : null };
  }
  if (!idOk) {
    return { ok: false, error: "missing id", id: null };
  }
  return {
    ok: true,
    request: { id: id as RequestId, method: obj["method"], params: obj["params"] },
  };
}

// ── Writer ───────────────────────────────────────────────────────────────────

export interface LineSink {
  write(chunk: string): unknown;
}

/**
 * Serialises protocol frames as single-line JSON. `JSON.stringify` never emits
 * raw newlines, so one frame is always exactly one line.
 */
export class ProtocolWriter {
  constructor(private readonly sink: LineSink) {}

  result(id: RequestId, result: unknown): void {
    this.line({ id, result });
  }

  error(id: RequestId | null, error: WireError): void {
    this.line({ id, error });
  }

  event<E extends SidecarEventName>(event: E, data: SidecarEventMap[E]): void {
    this.line({ event, data });
  }

  private line(frame: Record<string, unknown>): void {
    this.sink.write(`${JSON.stringify(frame)}\n`);
  }
}

export function wireError(code: string, message: string, kind: string): WireError {
  return { code, message, kind };
}
