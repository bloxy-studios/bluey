/**
 * ModePrompt fragments: per-schema FIELD instructions appended to the mode's
 * own `systemInstructions`. A fragment says which fields to fill and which
 * section titles exist — never how to sound: voice and content come from the
 * response contract (`src/ai/prompts/system.ts`) and the mode's instructions.
 *
 * Every double-quoted string in a fragment is a section title and must be one
 * of `SECTION_TITLES[schemaId]` (`src/modes/schemas.ts`) — the provider-side
 * schema is a closed enum, so a title the fragment invents would be rejected
 * or silently dropped. `tests/unit/modes/prompts.test.ts` enforces this. All
 * prompt strings live here — never inline them in the builder.
 */

import type { ResponseSchemaId } from "@/lib/types";

export interface ModePrompt {
  schemaId: ResponseSchemaId;
  fragment: string;
}

export const MODE_PROMPTS: Record<ResponseSchemaId, ModePrompt> = {
  answer: {
    schemaId: "answer",
    fragment: [
      "Fields: `content` is the answer — markdown, answer first, ready to read or paste.",
      "Leave `sections` empty unless the answer has genuinely separate named parts.",
    ].join(" "),
  },
  "suggested-response": {
    schemaId: "suggested-response",
    fragment: [
      "Fields: `content` is exactly what I say next — the words themselves, nothing around them.",
      'Optional `sections`, one line each: "Why it works" and "Key point".',
    ].join(" "),
  },
  behavioral: {
    schemaId: "behavioral",
    fragment: [
      "Fields: `content` is the spoken answer — STAR-shaped inside (situation, task, action, result) with no labels showing, drawn only from my real background in the context; never label the STAR parts out loud.",
      '`sections`, one line each: "Story used" (the experience it draws on) and "Key point" (the one thing to land). Do not repeat the spoken answer in a section.',
    ].join(" "),
  },
  coding: {
    schemaId: "coding",
    fragment: [
      "Fields: `content` opens with the approach in two to five lines, then the complete runnable solution in a fenced block with the language tag.",
      "`code` is that same full solution with `language` set (infer it from the visible editor or judge; never truncate or elide code).",
      '`sections`: "Complexity" (time and space, one line each with the reason) and "Edge cases" (the inputs that break naive solutions and how the code handles them).',
      "If the statement is incomplete, solve the most reasonable reading and state the assumption in one line.",
    ].join(" "),
  },
  "system-design": {
    schemaId: "system-design",
    fragment: [
      "Fields: `content` is the headline design in a few lines — the shape of the system and the two decisions that matter most.",
      '`sections` in this order: "Requirements", "Capacity estimates" (arithmetic visible), "High-level design", "Data model", "Deep dive", "Trade-offs".',
      "Optional `diagram`: a Mermaid graph of the high-level design.",
    ].join(" "),
  },
  case: {
    schemaId: "case",
    fragment: [
      "Fields: `content` is the recommendation or the next thing I say in the case, answer first.",
      '`sections` only for the stages in play right now: "Clarify", "Framework", "Analyze", "Calculate" (kind `calculation`: assumptions, then steps, then the result), "Synthesize", "Recommend".',
    ].join(" "),
  },
  sales: {
    schemaId: "sales",
    fragment: [
      "Fields: `content` is exactly what I say to the prospect next.",
      '`sections`, one line each: "Why it works" and "Optional follow-up" (one question). Do not repeat the spoken line in a section.',
    ].join(" "),
  },
  recruiting: {
    schemaId: "recruiting",
    fragment: [
      "Fields: `content` is what I say to the candidate next, or the next screening question to ask.",
      '`sections`: "Screening notes" (signals heard so far, one line each) and "Next step".',
    ].join(" "),
  },
  meeting: {
    schemaId: "meeting",
    fragment: [
      "Fields: `content` carries the callout or the recap itself.",
      'Live callouts as `sections` titled "Important", "Decision detected", "Action item detected" (task — owner — deadline) and "Question detected" — only the ones that apply.',
      'A recap as `sections` titled "Summary", "Decisions", "Action items" (owner and deadline when stated) and "Open questions".',
    ].join(" "),
  },
  lecture: {
    schemaId: "lecture",
    fragment: [
      "Fields: `content` is the explanation or the notes asked for.",
      '`sections` from "Concept", "Definition", "Example", "Notes" and "Questions" (likely exam questions, each with a brief model answer) — only the ones that apply.',
    ].join(" "),
  },
};

export function modePromptFor(schemaId: ResponseSchemaId): ModePrompt {
  return MODE_PROMPTS[schemaId];
}
