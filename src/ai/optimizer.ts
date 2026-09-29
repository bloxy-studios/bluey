/**
 * Response optimizer: cleans the model output before it hits the HUD.
 *
 * Guarantees:
 *  - fenced code blocks are preserved verbatim (never reflowed or trimmed)
 *  - factual caveats and citations are preserved
 *  - filler openers and question-restating first sentences are stripped
 *    (never one naming an option, number or code; never for the shapes
 *    whose first sentence is the deliverable),
 *    duplicate paragraphs removed
 *  - length capped by style (concise ≈ 120 words of prose, code excluded) —
 *    never for spoken, written, code, design or summary shapes, never the
 *    first paragraph
 *  - spoken and written answers lose a leaked leading heading and a trailing
 *    "Why it works" rationale
 *  - `title` derived when missing; `code` populated for coding responses
 */

import type { AnswerShape, BlueyMode, BlueyResponse, CodeBlock, ResponseStyle } from "@/lib/types";

const FILLER_OPENERS = [
  /^certainly[!,.]\s*/i,
  /^sure(?: thing)?[!,.]\s*/i,
  /^of course[!,.]\s*/i,
  /^absolutely[!,.]\s*/i,
  /^great question[!,.]?\s*/i,
  /^good question[!,.]?\s*/i,
  /^happy to help[!,.]?\s*/i,
  /^i'd be (?:glad|happy) to(?: help)?[!,.]?\s*/i,
  /^here(?:'s| is) (?:the|your|a|an) [^\n.:!]{0,40}?[:!]\s*/i,
  /^as an ai(?: language model)?,?\s*/i,
];

/**
 * Whole first sentences that restate the question or narrate the approach
 * instead of answering ("The question is asking about…", "Let's break this
 * down.", "Looking at the screen, …"). Each pattern ends at the sentence's
 * punctuation; a sentence is only removed when an answer remains after it.
 */
const RESTATEMENT_OPENERS = [
  /^(?:the|this|your) (?:question|prompt|task|problem|screen|screenshot|image|code|snippet|passage|text|error)(?: here| above| shown| below)? (?:is asking|asks|wants|requires)\b[^.!?:\n]*[.!?:]\s*/i,
  /^(?:you(?:'re| are) (?:asking|looking at|being asked)|you want to know|you asked|you'?d like to know)\b[^.!?:\n]*[.!?:]\s*/i,
  // Approach preambles end at their first comma or colon ("Looking at the screen, …").
  /^(?:to answer (?:this|your|the) question|to solve this|in order to answer|looking at (?:the|this|your) (?:screen|question|code|image|problem|options|error)|based on (?:the|your|this) (?:screen|screenshot|image|question|context|information provided))\b[^.!?,:\n]*[,.:!]\s*/i,
  /^(?:let'?s|let me) (?:break|walk|look|dive|begin|see|analy[sz]e|think)\b[^.!?:\n]*[.!?:]\s*/i,
  /^i(?:'ll| will| can)(?: help| explain| walk| break)\b[^.!?:\n]*[.!?:]\s*/i,
];

/** A sentence naming an option, a number or code carries the answer: never stripped. */
const ANSWER_MARKERS = /\b[A-E]\b|\boption\b|\d|`/;

/** Shapes whose first sentence is the deliverable: only exact filler is stripped. */
const VERBATIM_SHAPES: ReadonlySet<AnswerShape> = new Set<AnswerShape>(["spoken", "written", "code", "choice", "debug"]);

const MIN_WORDS_AFTER_STRIP = 3;

const CAVEAT_MARKERS =
  /\b(note:|caveat|important:|warning:|as of |i'?m not (fully )?(sure|certain)|may (be|have) (changed|outdated)|not (fully )?verified|double[- ]check|according to)\b/i;

const CITATION_MARKER = /\[\d+\]|\[\^?\w+\]|\bhttps?:\/\/\S+/;

interface Piece {
  kind: "prose" | "code";
  text: string;
}

/** Split markdown into prose and fenced-code pieces (code kept verbatim). */
export function splitCodeBlocks(markdown: string): Piece[] {
  const pieces: Piece[] = [];
  const fence = /^\s{0,3}```/;
  const lines = markdown.split("\n");
  let buffer: string[] = [];
  let inCode = false;

  const flush = (kind: Piece["kind"]) => {
    if (buffer.length === 0) return;
    pieces.push({ kind, text: buffer.join("\n") });
    buffer = [];
  };

  for (const line of lines) {
    if (fence.test(line)) {
      if (!inCode) {
        flush("prose");
        inCode = true;
        buffer.push(line);
      } else {
        buffer.push(line);
        flush("code");
        inCode = false;
      }
      continue;
    }
    buffer.push(line);
  }
  flush(inCode ? "code" : "prose");
  return pieces;
}

function normalizeParagraph(paragraph: string): string {
  return paragraph.toLowerCase().replace(/\s+/g, " ").trim();
}

function dedupeParagraphs(prose: string, seen: Set<string>): string {
  const paragraphs = prose.split(/\n{2,}/);
  const kept: string[] = [];
  for (const paragraph of paragraphs) {
    const normalized = normalizeParagraph(paragraph);
    if (normalized.length === 0) {
      continue;
    }
    if (seen.has(normalized)) continue;
    seen.add(normalized);
    kept.push(dedupeAdjacentSentences(paragraph.trim()));
  }
  return kept.join("\n\n");
}

/** Remove immediately-repeated sentences inside a paragraph. */
export function dedupeAdjacentSentences(paragraph: string): string {
  const sentences = paragraph.split(/(?<=[.!?])\s+/);
  const kept: string[] = [];
  let previous = "";
  for (const sentence of sentences) {
    const normalized = normalizeParagraph(sentence);
    if (normalized.length > 0 && normalized === previous) continue;
    kept.push(sentence);
    previous = normalized;
  }
  return kept.join(" ");
}

function countWords(text: string): number {
  return text.split(/\s+/).filter((w) => w.length > 0).length;
}

function recapitalize(text: string): string {
  if (text.length > 0 && /[a-z]/.test(text[0] ?? "")) {
    return (text[0] ?? "").toUpperCase() + text.slice(1);
  }
  return text;
}

/**
 * Strip filler openers ("Sure!", "Great question!") and restating first
 * sentences ("The question is asking…"). A restatement is only removed when
 * it names no option, number or code and at least a few words of answer
 * remain; spoken, written, code, choice and debug answers keep theirs (AI-003).
 */
export function stripFillerOpeners(text: string, shape?: AnswerShape): string {
  let result = text.trimStart();
  let changed = true;
  while (changed) {
    changed = false;
    for (const pattern of FILLER_OPENERS) {
      const next = result.replace(pattern, "");
      if (next !== result) {
        result = next.trimStart();
        changed = true;
      }
    }
  }
  changed = shape === undefined || !VERBATIM_SHAPES.has(shape);
  while (changed) {
    changed = false;
    for (const pattern of RESTATEMENT_OPENERS) {
      const removed = pattern.exec(result)?.[0] ?? "";
      if (!removed || ANSWER_MARKERS.test(removed)) continue;
      const next = result.slice(removed.length).trimStart();
      if (countWords(next) >= MIN_WORDS_AFTER_STRIP) {
        result = next;
        changed = true;
      }
    }
  }
  // Re-capitalize after stripping an opener mid-sentence.
  return recapitalize(result);
}

const LENGTH_WORD_CAPS: Record<ResponseStyle["length"], number> = {
  concise: 120,
  balanced: 320,
  detailed: 900,
};

/** Shapes whose text is the deliverable itself — cutting them mid-way would destroy it. */
const UNCAPPED_SHAPES: ReadonlySet<AnswerShape> = new Set<AnswerShape>([
  "spoken",
  "written",
  "code",
  "design",
  "summary",
]);

function mustKeepParagraph(paragraph: string): boolean {
  return CAVEAT_MARKERS.test(paragraph) || CITATION_MARKER.test(paragraph);
}

/**
 * Cap prose length at a word budget, keeping whole paragraphs. The first
 * paragraph (the answer) is always kept; paragraphs containing caveats or
 * citations are always kept; code is never counted or touched.
 */
export function capProse(pieces: Piece[], maxWords: number): Piece[] {
  let words = 0;
  return pieces.map((piece) => {
    if (piece.kind === "code") return piece;
    const paragraphs = piece.text.split(/\n{2,}/).filter((p) => p.trim().length > 0);
    const kept: string[] = [];
    for (const paragraph of paragraphs) {
      const cost = countWords(paragraph);
      if (words + cost <= maxWords || kept.length === 0 || mustKeepParagraph(paragraph)) {
        kept.push(paragraph);
        words += cost;
      }
    }
    return { kind: piece.kind, text: kept.join("\n\n") } as Piece;
  });
}

/** First fenced code block in the markdown, if any. */
export function firstCodeBlock(markdown: string): CodeBlock | null {
  const match = markdown.match(/^\s{0,3}```([a-zA-Z0-9_+-]*)\s*\n([\s\S]*?)\n\s{0,3}```\s*$/m);
  if (!match) return null;
  return { language: match[1] && match[1].length > 0 ? match[1] : "text", code: match[2] ?? "" };
}

export function deriveTitle(content: string, prompt?: string): string | undefined {
  const heading = content.match(/^#{1,3}\s+(.+)$/m);
  if (heading?.[1]) return heading[1].trim().slice(0, 60);
  const promptLine = prompt?.trim();
  if (promptLine && promptLine.length > 0) {
    return promptLine.length > 60 ? `${promptLine.slice(0, 57)}…` : promptLine;
  }
  const firstProse = content
    .split("\n")
    .map((l) => l.trim())
    .find((l) => l.length > 0 && !l.startsWith("```") && !l.startsWith("#"));
  if (!firstProse) return undefined;
  const sentence = firstProse.split(/(?<=[.!?])\s/)[0] ?? firstProse;
  return sentence.length > 60 ? `${sentence.slice(0, 57)}…` : sentence;
}

export interface OptimizeOptions {
  style: ResponseStyle;
  mode: BlueyMode;
  /** The detected answer shape; spoken, written, code, design and summary answers are never length-capped. */
  shape?: AnswerShape;
}

/** Shapes whose content is only the words to say or send (AI-012). */
const WORDS_ONLY_SHAPES: ReadonlySet<AnswerShape> = new Set<AnswerShape>(["spoken", "written"]);
const LEADING_HEADINGS = /^(?:\s*#{1,6}\s[^\n]*\n+)+/;
const TRAILING_RATIONALE = /\n{2,}(?:#{1,6}\s*|\*\*)?why (?:it|this) works\b[\s\S]*$/i;

/** Drop section scaffolding that leaked into a spoken or written answer. */
export function stripSpokenScaffolding(content: string): string {
  const stripped = content.replace(LEADING_HEADINGS, "").replace(TRAILING_RATIONALE, "").trim();
  return stripped.length > 0 ? stripped : content;
}

/** Clean and normalize a final response. Pure — returns a new object. */
export function optimizeResponse(response: BlueyResponse, opts: OptimizeOptions): BlueyResponse {
  const wordsOnly = opts.shape !== undefined && WORDS_ONLY_SHAPES.has(opts.shape);
  const pieces = splitCodeBlocks(wordsOnly ? stripSpokenScaffolding(response.content) : response.content);
  const hasCode = pieces.some((p) => p.kind === "code");

  const seen = new Set<string>();
  let cleaned: Piece[] = pieces.map((piece) =>
    piece.kind === "code" ? piece : { kind: piece.kind, text: dedupeParagraphs(piece.text, seen) },
  );

  // Strip filler from the first prose piece only (openers live there).
  const firstProseIndex = cleaned.findIndex((p) => p.kind === "prose" && p.text.trim().length > 0);
  if (firstProseIndex >= 0) {
    const piece = cleaned[firstProseIndex];
    if (piece) {
      cleaned[firstProseIndex] = { kind: "prose", text: stripFillerOpeners(piece.text, opts.shape) };
    }
  }

  // Cap prose length by style — never when the payload is primarily code, and
  // never for the shapes whose text or structure is the deliverable.
  const cap = LENGTH_WORD_CAPS[opts.style.length];
  const uncapped = (hasCode && opts.style.length === "concise") || (opts.shape !== undefined && UNCAPPED_SHAPES.has(opts.shape));
  if (!uncapped) {
    cleaned = capProse(cleaned, cap);
  }

  let content = cleaned
    .map((p) => p.text)
    .filter((t) => t.trim().length > 0)
    .join("\n\n");
  // Collapse >2 consecutive blank lines.
  content = content.replace(/\n{3,}/g, "\n\n").trim();

  const optimized: BlueyResponse = { ...response, content };

  if (!optimized.title) {
    const title = deriveTitle(content, response.prompt);
    if (title) optimized.title = title;
  }

  const isCoding = response.type === "code" || opts.mode.responseSchema === "coding";
  if (isCoding && !optimized.code) {
    const block =
      firstCodeBlock(content) ??
      (response.sections ?? [])
        .filter((s) => s.kind === "code" && s.content.trim().length > 0)
        .map((s) => ({ language: s.language ?? "text", code: s.content }))[0] ??
      null;
    if (block) optimized.code = block;
  }

  return optimized;
}
