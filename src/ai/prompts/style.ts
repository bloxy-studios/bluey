/**
 * Response style (length + tone) prompt fragments. Lengths are CEILINGS: a
 * one-line answer is complete under every setting, and nothing here asks the
 * model to fill the space (the response contract in `system.ts` has
 * precedence).
 */

import type { ResponseLength, ResponseStyle, ResponseTone } from "@/lib/types";

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
  direct: "Tone: direct — the answer, zero hedging, zero filler.",
};

export function styleBlock(style: ResponseStyle): string {
  return `${LENGTH_LINES[style.length]}\n${TONE_LINES[style.tone]}`;
}
