/**
 * Response optimizer: cleans the model output before it hits the HUD.
 *
 * Guarantees:
 *  - fenced code blocks are preserved verbatim (never reflowed or trimmed)
 *  - factual caveats and citations are preserved
 *  - filler openers are stripped, duplicate paragraphs removed
 *  - length capped by style (concise ≈ 120 words of prose, code excluded)
 *  - `title` derived when missing; `code` populated for coding responses
 */

import type { BlueyMode, BlueyResponse, CodeBlock, ResponseStyle } from "@/lib/types";

const FILLER_OPENERS = [
  /^certainly[!,.]?\s*/i,
  /^sure(?: thing)?[!,.]?\s*/i,
  /^of course[!,.]?\s*/i,
  /^absolutely[!,.]?\s*/i,
  /^great question[!,.]?\s*/i,
  /^good question[!,.]?\s*/i,
  /^happy to help[!,.]?\s*/i,
  /^i'd be (?:glad|happy) to(?: help)?[!,.]?\s*/i,
  /^here(?:'s| is) (?:the|your|a|an) [^\n.:!]{0,40}?[:!]\s*/i,
  /^as an ai(?: language model)?,?\s*/i,
];

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

export function stripFillerOpeners(text: string): string {
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
  // Re-capitalize after stripping an opener mid-sentence.
  if (result.length > 0 && /[a-z]/.test(result[0] ?? "")) {
    result = (result[0] ?? "").toUpperCase() + result.slice(1);
  }
  return result;
}

function countWords(text: string): number {
  return text.split(/\s+/).filter((w) => w.length > 0).length;
}

const LENGTH_WORD_CAPS: Record<ResponseStyle["length"], number> = {
  concise: 120,
  balanced: 320,
  detailed: 900,
};

function mustKeepParagraph(paragraph: string): boolean {
  return CAVEAT_MARKERS.test(paragraph) || CITATION_MARKER.test(paragraph);
}

/**
 * Cap prose length at a word budget, keeping whole paragraphs. Paragraphs
 * containing caveats or citations are always kept; code is never counted or
 * touched.
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
}

/** Clean and normalize a final response. Pure — returns a new object. */
export function optimizeResponse(response: BlueyResponse, opts: OptimizeOptions): BlueyResponse {
  const pieces = splitCodeBlocks(response.content);
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
      cleaned[firstProseIndex] = { kind: "prose", text: stripFillerOpeners(piece.text) };
    }
  }

  // Cap prose length by style — never when the payload is primarily code.
  const cap = LENGTH_WORD_CAPS[opts.style.length];
  if (!(hasCode && opts.style.length === "concise")) {
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
