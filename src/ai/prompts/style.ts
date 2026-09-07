/** Response style (length + tone) prompt fragments. */

import type { ResponseLength, ResponseStyle, ResponseTone } from "@/lib/types";

const LENGTH_LINES: Record<ResponseLength, string> = {
  concise:
    "Length: concise. Aim for under 120 words of prose (code and calculations excluded). One idea per sentence. Cut preamble entirely.",
  balanced:
    "Length: balanced. A few short paragraphs or a tight list — enough to be complete, nothing ornamental.",
  detailed:
    "Length: detailed. Cover the topic thoroughly with structure (headings/lists), but stay information-dense — no padding.",
};

const TONE_LINES: Record<ResponseTone, string> = {
  natural: "Tone: natural — like a sharp colleague whispering the right answer. Contractions fine.",
  professional: "Tone: professional — polished and composed, suitable to repeat verbatim in a business setting.",
  technical: "Tone: technical — precise terminology, no simplification for its own sake.",
  conversational: "Tone: conversational — relaxed, spoken rhythm, first person.",
  direct: "Tone: direct — lead with the answer, zero hedging, zero filler.",
};

export function styleBlock(style: ResponseStyle): string {
  return `${LENGTH_LINES[style.length]}\n${TONE_LINES[style.tone]}`;
}
