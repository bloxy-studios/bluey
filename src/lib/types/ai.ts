/**
 * AI request/stream contract between the TS intelligence layer and the Rust
 * provider layer. Rust owns credentials + HTTP; TS owns prompts + policy.
 * Mirrors `bluey_core::types::ai`.
 */

import type { TraceStamps } from "./latency";
import type { BlueyError } from "./errors";
import type { ModelRole } from "./mode";
import type { Citation } from "./response";

export type AITask =
  | "vision"
  | "classification"
  | "transcription"
  | "answer"
  | "coding"
  | "system_design"
  | "summarization"
  | "research"
  | "deep_reasoning"
  | "embedding";

/**
 * The shape the answer should take — detected from the question and the
 * screen (`classifyIntent`), rendered as the `Shape:` line after `Task:` and
 * used for output budgets. Orthogonal to `AITask` (what kind of work) and to
 * the response schema (which fields come back). Never sent to Rust.
 */
export type AnswerShape =
  | "choice" // multiple choice: the option, one clause of why
  | "boolean" // yes/no, true/false
  | "fill_in" // fill in the blank
  | "calculation" // the result, then the working
  | "compare" // which of two is better and why
  | "short_answer" // one to three sentences
  | "explain" // answer first, then the reasons or steps
  | "spoken" // exactly what to say, first person
  | "written" // the text to send or submit
  | "code" // the working solution
  | "design" // a system design with its trade-offs
  | "summary"; // the points themselves

export type LatencyBudget = "ultra-fast" | "fast" | "balanced" | "deep";
export type ReasoningLevel = "none" | "light" | "deep";

/**
 * `google_gemini` is the default provider (ADR 0007); the others are alternates.
 * `chatgpt_codex`, `claude_subscription` and `antigravity_google` are served by
 * subscription accounts rather than API keys (ADR 0009; reserved ids `chatgpt`,
 * `claude`, `antigravity`).
 */
export type AIProviderKind =
  | "google_gemini"
  | "azure_foundry"
  | "anthropic"
  | "openai_compatible"
  | "chatgpt_codex"
  | "claude_subscription"
  | "antigravity_google"
  | "mock";

/** What texts are embedded for (documents get `title:` prefixes, queries `task:` prefixes on gemini-embedding-2). */
export type EmbedPurpose = "document" | "query";

export interface AIProviderConfig {
  id: string;
  kind: AIProviderKind;
  name: string;
  /** e.g. https://my-resource.openai.azure.com or https://api.anthropic.com */
  baseUrl: string;
  /** Azure api-version when using the legacy deployment endpoint. */
  apiVersion?: string;
  /** For Azure: map of logical model name -> deployment name (optional). */
  deployments?: Record<string, string>;
  enabled: boolean;
  /** True when a key is stored in the OS keychain (never the key itself). */
  hasApiKey: boolean;
  /** API key (default when absent) or an OAuth subscription account (ADR 0009). */
  authMethod?: "api_key" | "oauth_subscription";
}

export interface ModelAssignment {
  providerId: string;
  model: string;
}

export type ModelRoleAssignments = Record<ModelRole, ModelAssignment | null>;

export type AIContentPart =
  | { type: "text"; text: string }
  | { type: "image"; mediaType: "image/jpeg" | "image/png" | "image/webp"; data: string };

export interface AIMessage {
  role: "system" | "user" | "assistant";
  content: AIContentPart[];
}

export interface JsonSchemaSpec {
  name: string;
  schema: Record<string, unknown>;
  strict?: boolean;
}

export interface AIRequest {
  requestId: string;
  sessionId?: string;
  /** Monotonic per-window generation for stale-response protection. */
  generation: number;
  task: AITask;
  latencyBudget: LatencyBudget;
  reasoning: ReasoningLevel;
  visionRequired: boolean;
  /** Estimated input tokens (helps routing). */
  contextTokens: number;
  messages: AIMessage[];
  outputSchema?: JsonSchemaSpec;
  maxOutputTokens?: number;
  temperature?: number;
  /** Explicit override of the router decision. */
  modelOverride?: ModelAssignment;
  /** The WebView's half of the fast-path trace (ADR 0010 §2), when the engine measured it. */
  trace?: TraceStamps;
  createdAt: string;
}

export interface ModelSelection {
  providerId: string;
  providerKind: AIProviderKind;
  model: string;
  role: ModelRole;
  reason: string;
}

export type AIChunk =
  | { type: "started"; requestId: string; selection: ModelSelection }
  | { type: "delta"; requestId: string; text: string }
  | { type: "usage"; requestId: string; inputTokens?: number; outputTokens?: number }
  | {
      type: "completed";
      requestId: string;
      finishReason: "stop" | "length" | "cancelled" | "error";
      totalMs: number;
      timeToFirstTokenMs?: number;
    }
  | { type: "failed"; requestId: string; error: BlueyError };

export interface AIResponse {
  requestId: string;
  selection: ModelSelection;
  text: string;
  /** Parsed JSON when `outputSchema` was requested and parsing succeeded. */
  json?: unknown;
  finishReason: "stop" | "length" | "cancelled" | "error";
  inputTokens?: number;
  outputTokens?: number;
  timeToFirstTokenMs?: number;
  totalMs: number;
}

export interface ConnectionTestResult {
  ok: boolean;
  providerId: string;
  model?: string;
  latencyMs?: number;
  error?: BlueyError;
}

/** Research */
export type ResearchDepth = "none" | "search" | "search_scrape" | "deep_agent";

export interface SearchResult {
  id: string;
  title: string;
  url: string;
  snippet?: string;
  publishedAt?: string;
  source: "exa" | "firecrawl" | "mock";
}

export interface ScrapeResult {
  url: string;
  title?: string;
  markdown: string;
  source: "firecrawl" | "mock";
}

export interface DeepResearchRequest {
  jobId: string;
  sessionId?: string;
  /** PUBLIC query only — never include private context. */
  query: string;
  goal: string;
  maxTurns?: number;
  tools: Array<"exa_search" | "firecrawl_scrape" | "document_read">;
  /** Document ids the agent may read via the document_read tool (local, private). */
  allowedDocumentIds?: string[];
}

export type DeepResearchEvent =
  | { type: "started"; jobId: string }
  | { type: "progress"; jobId: string; message: string }
  | { type: "tool_call"; jobId: string; tool: string; input: Record<string, unknown> }
  | { type: "text_delta"; jobId: string; text: string }
  | {
      type: "completed";
      jobId: string;
      report: string;
      citations: Citation[];
      totalMs: number;
      turns: number;
    }
  | { type: "failed"; jobId: string; error: BlueyError };
