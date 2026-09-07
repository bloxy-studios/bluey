/**
 * ModePrompt fragments: per-schema shaping instructions appended to the mode's
 * own `systemInstructions`. All prompt strings live here — never inline them
 * in the builder.
 */

import type { ResponseSchemaId } from "@/lib/types";

export interface ModePrompt {
  schemaId: ResponseSchemaId;
  fragment: string;
}

const NEVER_FABRICATE =
  "Never fabricate facts, experience, metrics or citations. If the context does not contain something, say so briefly instead of inventing it.";

export const MODE_PROMPTS: Record<ResponseSchemaId, ModePrompt> = {
  answer: {
    schemaId: "answer",
    fragment: [
      "Answer the question directly in the first sentence, then add only the detail that earns its place.",
      "Prefer short paragraphs and tight lists over walls of text.",
      NEVER_FABRICATE,
    ].join(" "),
  },
  "suggested-response": {
    schemaId: "suggested-response",
    fragment: [
      "Draft exactly what the user should SAY next, in their voice, first person, ready to speak aloud.",
      "No stage directions, no 'you could say'. It must sound natural when read verbatim.",
      'Optionally add short sections titled "Why it works" and "Key point".',
      NEVER_FABRICATE,
    ].join(" "),
  },
  behavioral: {
    schemaId: "behavioral",
    fragment: [
      "Draft a spoken answer to the behavioral question grounded ONLY in the user's real background from the provided context.",
      "Structure it internally as STAR (situation, task, action, result) but keep the delivery natural and conversational — never label the STAR parts out loud.",
      'Return sections titled "Suggested answer", "Story used" (which experience it draws on) and "Key point".',
      NEVER_FABRICATE,
    ].join(" "),
  },
  coding: {
    schemaId: "coding",
    fragment: [
      "Solve the coding problem visible in the context.",
      'Return sections titled "Approach", "Solution" (kind "code" with the language set), "Complexity" and "Edge cases".',
      "Also set the top-level `code` field to the full working solution. State time/space complexity explicitly.",
      "If the problem statement is incomplete, solve the most reasonable interpretation and say what you assumed.",
      NEVER_FABRICATE,
    ].join(" "),
  },
  "system-design": {
    schemaId: "system-design",
    fragment: [
      "Work the system design question like a strong senior engineer at a whiteboard.",
      'Return sections in this order: "Requirements", "Capacity estimates", "High-level design", "Data model", "Deep dive", "Trade-offs".',
      "Optionally include a `diagram` field with a Mermaid graph of the high-level architecture.",
      "Quantify estimates (QPS, storage, bandwidth) with visible arithmetic.",
      NEVER_FABRICATE,
    ].join(" "),
  },
  case: {
    schemaId: "case",
    fragment: [
      "Coach the user through the case interview.",
      'Return sections titled "Clarify", "Framework", "Analyze", "Calculate" (kind "calculation" with visible arithmetic), "Synthesize" and "Recommend".',
      "Be hypothesis-driven and quantitative; round numbers the way a candidate would out loud.",
      NEVER_FABRICATE,
    ].join(" "),
  },
  sales: {
    schemaId: "sales",
    fragment: [
      "Help the user advance the deal in this live conversation.",
      'Return sections titled "Suggested response" (first person, ready to say), "Why it works" and "Optional follow-up".',
      "Acknowledge objections honestly — never dismiss them, never over-promise, never invent product claims.",
      NEVER_FABRICATE,
    ].join(" "),
  },
  recruiting: {
    schemaId: "recruiting",
    fragment: [
      "Support the user as the recruiter/interviewer in this conversation.",
      'Return sections titled "Suggested response" (first person, ready to say), "Screening notes" (signals heard so far) and "Next step".',
      "Stay factual about the role using the job description context; flag mismatches neutrally.",
      NEVER_FABRICATE,
    ].join(" "),
  },
  meeting: {
    schemaId: "meeting",
    fragment: [
      "During a live meeting, surface only what matters right now.",
      'For live updates use sections titled "Important", "Decision detected", "Action item detected" and "Question detected" — include only the ones that apply.',
      'For a post-meeting summary use sections titled "Summary", "Decisions", "Action items" (with owner and deadline when stated) and "Open questions".',
      NEVER_FABRICATE,
    ].join(" "),
  },
  lecture: {
    schemaId: "lecture",
    fragment: [
      "Turn the lecture content into crisp study material.",
      'Return sections titled "Concept", "Definition", "Example", "Notes" and "Questions" (good questions to ask or expect on an exam) — include the ones that apply.',
      "Define terms precisely; keep examples concrete and small.",
      NEVER_FABRICATE,
    ].join(" "),
  },
};

export function modePromptFor(schemaId: ResponseSchemaId): ModePrompt {
  return MODE_PROMPTS[schemaId];
}
