/** Structured AI response contract (mirrors `bluey_core::types::response`). */

import type { ModelRole } from "./mode";

export type ResponseType = "answer" | "code" | "system-design" | "summary" | "suggestion" | "research";

export interface ResponseSection {
  id: string;
  title: string;
  content: string;
  /** Rendering hint. */
  kind?: "text" | "list" | "code" | "diagram" | "table" | "calculation";
  language?: string;
  collapsed?: boolean;
}

export interface Citation {
  id: string;
  title: string;
  url: string;
  snippet?: string;
}

export interface CodeBlock {
  language: string;
  code: string;
  filename?: string;
}

export type FeedbackRating = "up" | "down";
export type FeedbackCategory = "wrong" | "too_long" | "not_relevant" | "missed_context" | "wrong_tone";

export interface ResponseFeedback {
  responseId: string;
  rating: FeedbackRating;
  categories?: FeedbackCategory[];
  comment?: string;
  createdAt: string;
}

/**
 * Which model answered (from the router's `started` chunk), for provenance in the HUD.
 * In-memory only: not part of the persisted response row.
 */
export interface ResponseSelection {
  role: ModelRole;
  providerId: string;
  /** The provider's display name, when it is a configured provider. */
  providerName?: string;
  model: string;
  /** The router's explanation when the preferred role had no usable model and it fell back. */
  fallbackReason?: string;
}

export interface ResponseMetrics {
  provider?: string;
  model?: string;
  captureMs?: number;
  ocrMs?: number;
  accessibilityMs?: number;
  contextAssemblyMs?: number;
  timeToFirstTokenMs?: number;
  totalMs?: number;
  inputTokens?: number;
  outputTokens?: number;
  contextTokens?: number;
}

export interface BlueyResponse {
  id: string;
  requestId: string;
  sessionId?: string;
  modeId: string;
  type: ResponseType;
  title?: string;
  /** Markdown body. Code blocks inside are preserved verbatim. */
  content: string;
  code?: CodeBlock;
  sections?: ResponseSection[];
  citations?: Citation[];
  confidence?: number;
  /** The user instruction or detected question this responds to. */
  prompt?: string;
  /** Mermaid diagram source when a system-design response includes one. */
  diagram?: string;
  metrics?: ResponseMetrics;
  feedback?: ResponseFeedback;
  /** True when this response was prepared proactively and not yet shown. */
  prepared?: boolean;
  /** Short notice that web research failed and the answer went without it. */
  researchNote?: string;
  /**
   * True when the output budget cut the answer short (after the engine's one
   * retry with double the room): `content` is what could be salvaged and the
   * HUD shows "Answer was cut short" with Regenerate under it.
   */
  truncated?: boolean;
  /** Which model answered (in-memory provenance for the HUD; not persisted). */
  selection?: ResponseSelection;
  /** A short note about the research step (e.g. why web research was skipped), when there is one. */
  researchNote?: string;
  createdAt: string;
}

/** The raw structured object we ask models to return (provider structured output). */
export interface StructuredModelOutput {
  responseType: ResponseType;
  title?: string;
  content: string;
  sections?: Array<Omit<ResponseSection, "id">>;
  code?: CodeBlock;
  diagram?: string;
  confidence?: number;
  citations?: Array<Omit<Citation, "id">>;
  /** True when the envelope was incomplete and `content` was read out of the partial JSON. */
  salvaged?: boolean;
}
