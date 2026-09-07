/** Structured AI response contract (mirrors `bluey_core::types::response`). */

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
}
