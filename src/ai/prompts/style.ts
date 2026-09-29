/**
 * Response style (length + tone) prompt fragments. Lengths are CEILINGS: a
 * one-line answer is complete under every setting, and nothing here asks the
 * model to fill the space (the response contract in `system.ts` has
 * precedence).
 */

import type { AnswerShape, ResponseLength, ResponseStyle, ResponseTone } from "@/lib/types";

const LENGTH_LINES: Record<ResponseLength, string> = {
  concise:
    "Length ceiling: concise — at most ~120 words of prose (code and calculations excluded). Shorter is better whenever the shape allows it; a one-line answer is complete.",
  balanced:
    "Length ceiling: balanced — at most a few short paragraphs or a tight list. Stop the moment the answer is usable; never pad towards the ceiling.",
  detailed:
    "Length ceiling: detailed — structure (headings, lists) is welcome when the answer has parts, but every line must carry information. The ceiling is not a target.",
};

const TONE_LINES: Record<ResponseTone, string> = {
  natural: "Tone: natural — like a sharp colleague whispering the right answer. Contractions fine.",
  professional: "Tone: professional — polished and composed, suitable to repeat verbatim in a business setting.",
  technical: "Tone: technical — precise terminology, no simplification for its own sake.",
  conversational: "Tone: conversational — relaxed, spoken rhythm, first person.",
  direct: "Tone: direct — the answer, zero filler; real uncertainty in one clause at most.",
};

/** Shapes whose structure is the deliverable: no word ceiling, even under concise (AI-006). */
export const STRUCTURED_SHAPES: ReadonlySet<AnswerShape> = new Set<AnswerShape>(["design", "code", "summary"]);

const CONCISE_STRUCTURED =
  "Length ceiling: concise — every line must carry information; the shape's own parts (sections, code, points) set the length, not a word count.";

export function styleBlock(style: ResponseStyle, shape?: AnswerShape): string {
  const structured = style.length === "concise" && shape !== undefined && STRUCTURED_SHAPES.has(shape);
  return `${structured ? CONCISE_STRUCTURED : LENGTH_LINES[style.length]}\n${TONE_LINES[style.tone]}`;
}
