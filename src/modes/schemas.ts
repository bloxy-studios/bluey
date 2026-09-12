/**
 * Structured output schemas per `ResponseSchemaId`.
 *
 * Two layers:
 *  - STRICT provider schemas (zod → JSON Schema via `z.toJSONSchema`) that
 *    guide the model's structured output;
 *  - a TOLERANT parser that accepts what actually comes back: fences stripped,
 *    trailing commas repaired, double-encoded envelopes unwrapped, a cut-off
 *    envelope salvaged from its streamed `content`, and a plain-text fallback
 *    for prose — but never raw JSON: an envelope with nothing readable in it
 *    parses to null so the engine can show a failure state instead.
 */

import * as z from "zod";
import type {
  JsonSchemaSpec,
  ResponseSchemaId,
  ResponseType,
  StructuredModelOutput,
} from "@/lib/types";
import {
  extractCompleteStringField,
  extractPartialStringField,
  looksLikeStructuredJson,
} from "@/lib/utils/partial-json";

// ── zod building blocks ─────────────────────────────────────────────────────

const sectionKind = z.enum(["text", "list", "code", "diagram", "table", "calculation"]);

function sectionSchema(titles?: readonly string[]) {
  return z.object({
    title: titles && titles.length > 0 ? z.enum(titles as [string, ...string[]]) : z.string(),
    content: z.string(),
    kind: sectionKind.optional(),
    language: z.string().optional(),
  });
}

const codeBlockSchema = z.object({
  language: z.string(),
  code: z.string(),
  filename: z.string().optional(),
});

const citationSchema = z.object({
  title: z.string(),
  url: z.string(),
  snippet: z.string().optional(),
});

interface SchemaShapeOptions {
  responseType: ResponseType;
  sectionTitles?: readonly string[];
  includeCode?: boolean;
  includeDiagram?: boolean;
}

function structuredSchema(opts: SchemaShapeOptions) {
  return z.object({
    responseType: z.literal(opts.responseType),
    title: z.string().optional(),
    content: z.string(),
    sections: z.array(sectionSchema(opts.sectionTitles)).optional(),
    ...(opts.includeCode ? { code: codeBlockSchema.optional() } : {}),
    ...(opts.includeDiagram ? { diagram: z.string().optional() } : {}),
    confidence: z.number().min(0).max(1).optional(),
    citations: z.array(citationSchema).optional(),
  });
}

// ── Per-schema definitions ──────────────────────────────────────────────────

export const SECTION_TITLES: Partial<Record<ResponseSchemaId, readonly string[]>> = {
  "suggested-response": ["Why it works", "Key point"],
  behavioral: ["Suggested answer", "Story used", "Key point"],
  coding: ["Approach", "Solution", "Complexity", "Edge cases"],
  "system-design": [
    "Requirements",
    "Capacity estimates",
    "High-level design",
    "Data model",
    "Deep dive",
    "Trade-offs",
  ],
  case: ["Clarify", "Framework", "Analyze", "Calculate", "Synthesize", "Recommend"],
  sales: ["Suggested response", "Why it works", "Optional follow-up"],
  recruiting: ["Suggested response", "Screening notes", "Next step"],
  meeting: [
    "Important",
    "Decision detected",
    "Action item detected",
    "Question detected",
    "Summary",
    "Decisions",
    "Action items",
    "Open questions",
  ],
  lecture: ["Concept", "Definition", "Example", "Notes", "Questions"],
};

export const RESPONSE_TYPE_FOR_SCHEMA: Record<ResponseSchemaId, ResponseType> = {
  answer: "answer",
  "suggested-response": "suggestion",
  behavioral: "suggestion",
  coding: "code",
  "system-design": "system-design",
  case: "suggestion",
  sales: "suggestion",
  recruiting: "suggestion",
  meeting: "answer",
  lecture: "answer",
};

const ZOD_SCHEMAS: Record<ResponseSchemaId, z.ZodType> = {
  answer: structuredSchema({ responseType: "answer" }),
  "suggested-response": structuredSchema({
    responseType: "suggestion",
    sectionTitles: SECTION_TITLES["suggested-response"],
  }),
  behavioral: structuredSchema({ responseType: "suggestion", sectionTitles: SECTION_TITLES.behavioral }),
  coding: structuredSchema({
    responseType: "code",
    sectionTitles: SECTION_TITLES.coding,
    includeCode: true,
  }),
  "system-design": structuredSchema({
    responseType: "system-design",
    sectionTitles: SECTION_TITLES["system-design"],
    includeDiagram: true,
  }),
  case: structuredSchema({ responseType: "suggestion", sectionTitles: SECTION_TITLES.case }),
  sales: structuredSchema({ responseType: "suggestion", sectionTitles: SECTION_TITLES.sales }),
  recruiting: structuredSchema({ responseType: "suggestion", sectionTitles: SECTION_TITLES.recruiting }),
  meeting: structuredSchema({ responseType: "answer", sectionTitles: SECTION_TITLES.meeting }),
  lecture: structuredSchema({ responseType: "answer", sectionTitles: SECTION_TITLES.lecture }),
};

/** Provider-facing JSON Schema for a response schema id. */
export function outputSchemaFor(schemaId: ResponseSchemaId): JsonSchemaSpec {
  const schema = ZOD_SCHEMAS[schemaId];
  return {
    name: `bluey_${schemaId.replace(/-/g, "_")}`,
    schema: z.toJSONSchema(schema) as Record<string, unknown>,
    strict: true,
  };
}

// ── Tolerant parsing ────────────────────────────────────────────────────────

/**
 * Providers in strict structured-output mode (ChatGPT / Codex, Foundry,
 * OpenAI-compatible) are sent every optional field as a required *nullable*
 * one (`bluey_protocols::json_schema`), so `null` here means "absent".
 */
const undefinedIfNull = <T>(value: T | null | undefined): T | undefined => value ?? undefined;

const tolerantSection = z.object({
  title: z
    .string()
    .nullish()
    .transform((v) => v ?? ""),
  content: z
    .string()
    .nullish()
    .transform((v) => v ?? ""),
  kind: sectionKind.nullish().transform(undefinedIfNull),
  language: z.string().nullish().transform(undefinedIfNull),
  collapsed: z.boolean().nullish().transform(undefinedIfNull),
});

const tolerantCodeBlock = z.object({
  language: z.string(),
  code: z.string(),
  filename: z.string().nullish().transform(undefinedIfNull),
});

const tolerantCitation = z.object({
  title: z.string(),
  url: z.string(),
  snippet: z.string().nullish().transform(undefinedIfNull),
});

const RESPONSE_TYPES: readonly ResponseType[] = [
  "answer",
  "code",
  "system-design",
  "summary",
  "suggestion",
  "research",
];

const tolerantOutput = z.object({
  responseType: z.string().nullish().transform(undefinedIfNull),
  title: z.string().nullish().transform(undefinedIfNull),
  content: z.string().nullish().transform(undefinedIfNull),
  sections: z.array(z.unknown()).nullish().transform(undefinedIfNull),
  code: z.unknown().optional(),
  diagram: z.string().nullish().transform(undefinedIfNull),
  confidence: z.number().nullish().transform(undefinedIfNull),
  citations: z.array(z.unknown()).nullish().transform(undefinedIfNull),
});

/** Strip a wrapping markdown code fence (```json ... ```), if present. */
export function stripWrappingFence(text: string): string {
  const trimmed = text.trim();
  const match = trimmed.match(/^```[a-zA-Z0-9_-]*\s*\n([\s\S]*?)\n?```\s*$/);
  return match?.[1] !== undefined ? match[1] : trimmed;
}

/** Remove trailing commas before `}` or `]` (a common model slip). */
export function repairTrailingCommas(text: string): string {
  return text.replace(/,\s*([}\]])/g, "$1");
}

function extractFirstJsonObject(text: string): string | null {
  const start = text.indexOf("{");
  if (start === -1) return null;
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let i = start; i < text.length; i += 1) {
    const ch = text[i];
    if (inString) {
      if (escaped) escaped = false;
      else if (ch === "\\") escaped = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') inString = true;
    else if (ch === "{") depth += 1;
    else if (ch === "}") {
      depth -= 1;
      if (depth === 0) return text.slice(start, i + 1);
    }
  }
  return null;
}

/** Loose JSON parse: unfence, repair trailing commas, extract embedded object. */
export function parseJsonLoose(text: string): unknown | null {
  return tryParseJson(stripWrappingFence(text));
}

function tryParseJson(text: string): unknown | null {
  const candidates = [text, repairTrailingCommas(text)];
  const embedded = extractFirstJsonObject(text);
  if (embedded) candidates.push(embedded, repairTrailingCommas(embedded));
  for (const candidate of candidates) {
    try {
      return JSON.parse(candidate);
    } catch {
      // try the next repair
    }
  }
  return null;
}

function coerceSections(raw: unknown[] | undefined): StructuredModelOutput["sections"] {
  if (!raw) return undefined;
  const sections: NonNullable<StructuredModelOutput["sections"]> = [];
  for (const entry of raw) {
    const parsed = tolerantSection.safeParse(entry);
    if (parsed.success && (parsed.data.title.length > 0 || parsed.data.content.length > 0)) {
      sections.push(parsed.data);
    }
  }
  return sections.length > 0 ? sections : undefined;
}

function coerceCode(raw: unknown): StructuredModelOutput["code"] {
  const parsed = tolerantCodeBlock.safeParse(raw);
  return parsed.success ? parsed.data : undefined;
}

function coerceCitations(raw: unknown[] | undefined): StructuredModelOutput["citations"] {
  if (!raw) return undefined;
  const citations: NonNullable<StructuredModelOutput["citations"]> = [];
  for (const entry of raw) {
    const parsed = tolerantCitation.safeParse(entry);
    if (parsed.success) citations.push(parsed.data);
  }
  return citations.length > 0 ? citations : undefined;
}

/** Read a parsed envelope object; null when it holds nothing showable. */
function fromEnvelope(schemaId: ResponseSchemaId, json: object): StructuredModelOutput | null {
  const parsed = tolerantOutput.safeParse(json);
  if (!parsed.success) return null;
  const data = parsed.data;
  const responseType = RESPONSE_TYPES.includes(data.responseType as ResponseType)
    ? (data.responseType as ResponseType)
    : RESPONSE_TYPE_FOR_SCHEMA[schemaId];
  const sections = coerceSections(data.sections);
  let content =
    data.content && data.content.length > 0
      ? data.content
      : (sections ?? []).map((s) => `## ${s.title}\n${s.content}`).join("\n\n");

  // `content` that is itself an envelope (a double-encoded reply): unwrap it,
  // or salvage its own `content`; a JSON string is never handed to the HUD.
  let salvaged = false;
  if (looksLikeStructuredJson(content)) {
    const inner = tryParseJson(stripWrappingFence(content));
    if (inner !== null && typeof inner === "object") return fromEnvelope(schemaId, inner);
    const innerContent = extractPartialStringField(stripWrappingFence(content), "content")?.trim();
    if (!innerContent) return null;
    content = innerContent;
    salvaged = true;
  }

  if (content.length === 0 && !sections) return null;
  return {
    responseType,
    title: data.title,
    content,
    sections,
    code: coerceCode(data.code),
    diagram: data.diagram,
    confidence:
      typeof data.confidence === "number" ? Math.min(1, Math.max(0, data.confidence)) : undefined,
    citations: coerceCitations(data.citations),
    ...(salvaged ? { salvaged } : {}),
  };
}

/**
 * Parse model output into a `StructuredModelOutput`. Tolerant by design:
 * strips code fences, repairs trailing commas, extracts an embedded JSON
 * object, unwraps a double-encoded envelope, and — when the envelope was cut
 * off or malformed — salvages the streamed `content` (and a complete `title`).
 * Prose that was never JSON falls back to `{ responseType: "answer", content }`.
 * Returns null for empty output and for an envelope with nothing readable in
 * it: raw JSON is never returned as content.
 */
export function parseStructuredOutput(
  schemaId: ResponseSchemaId,
  text: string,
): StructuredModelOutput | null {
  if (text.trim().length === 0) return null;

  const unfenced = stripWrappingFence(text);
  let json = tryParseJson(unfenced);
  // The whole reply as one JSON string holding the envelope.
  if (typeof json === "string" && looksLikeStructuredJson(json)) {
    json = tryParseJson(stripWrappingFence(json));
  }
  if (json !== null && typeof json === "object") {
    const parsed = fromEnvelope(schemaId, json);
    if (parsed) return parsed;
    if (looksLikeStructuredJson(unfenced)) return null;
  }

  if (looksLikeStructuredJson(unfenced)) {
    // Cut off by the budget or malformed past repair: the `content` string
    // streamed before the break is the answer so far.
    const content = extractPartialStringField(unfenced, "content")?.trim();
    if (!content) return null;
    const title = extractCompleteStringField(unfenced, "title")?.trim();
    return {
      responseType: RESPONSE_TYPE_FOR_SCHEMA[schemaId],
      ...(title ? { title } : {}),
      content,
      salvaged: true,
    };
  }

  return { responseType: "answer", content: text.trim() };
}
