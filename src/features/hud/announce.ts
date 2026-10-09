import { presentError } from "@/lib/errors/present";
import type { BlueyResponse } from "@/lib/types";
import type { PillContent } from "./state-pill";

/** Longest answer summary read out; the full answer is in the thread. */
const SUMMARY_MAX_CHARS = 140;

/** What a pill state says out loud; states not worth interrupting for stay silent. */
export function pillAnnouncement(pill: PillContent): string | null {
  switch (pill.kind) {
    case "reading":
      return "Bluey is reading your screen";
    case "thinking":
      return "Bluey is thinking";
    case "researching":
      return "Bluey is researching";
    case "listening":
      return "Bluey is listening";
    case "prepared":
      return "Bluey has a suggestion";
    case "error":
      return `Error: ${pill.error ? presentError(pill.error).title : "Something went wrong"}`;
    default:
      return null;
  }
}

/** The answer's title, else its first sentence without markdown punctuation. */
export function answerSummary(response: BlueyResponse): string {
  if (response.title?.trim()) return response.title.trim();
  const plain = response.content
    .replace(/```[\s\S]*?(```|$)/g, " ")
    .replace(/[#*_`>|~[\]]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  const sentence = /^.+?[.!?](\s|$)/.exec(plain)?.[0]?.trim() ?? plain;
  return sentence.length > SUMMARY_MAX_CHARS ? `${sentence.slice(0, SUMMARY_MAX_CHARS - 1)}…` : sentence;
}
