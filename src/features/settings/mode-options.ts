import type { ContextRequirement, ResponseSchemaId } from "@/lib/types";

/** Response format picker (custom modes). */
export const RESPONSE_FORMAT_OPTIONS: Array<{ value: ResponseSchemaId; label: string }> = [
  { value: "answer", label: "Answer" },
  { value: "suggested-response", label: "Suggested response" },
  { value: "behavioral", label: "Behavioral (STAR)" },
  { value: "coding", label: "Coding" },
  { value: "system-design", label: "System design" },
  { value: "case", label: "Case interview" },
  { value: "sales", label: "Sales" },
  { value: "recruiting", label: "Recruiting" },
  { value: "meeting", label: "Meeting" },
  { value: "lecture", label: "Lecture" },
];

/**
 * Context-source chips: what a mode gathers before answering, and the stored
 * requirements each chip turns on or off. Every screen ask also reads the
 * accessibility tree, so Screen covers it; session memory applies in every mode,
 * so it has no chip (MODE-011).
 */
export const CONTEXT_SOURCE_CHIPS: ReadonlyArray<{
  source: ContextRequirement;
  label: string;
  /** Requirements the chip turns on and off with its source. */
  covers?: readonly ContextRequirement[];
}> = [
  { source: "screen", label: "Screen", covers: ["accessibility"] },
  { source: "transcript", label: "Transcript" },
  { source: "resume", label: "Résumé / CV" },
  { source: "job_description", label: "Job description" },
  { source: "documents", label: "Documents" },
];

/** `requirements` with a chip toggled; requirements without a chip are kept. */
export function toggleContextChip(
  requirements: readonly ContextRequirement[],
  chip: (typeof CONTEXT_SOURCE_CHIPS)[number],
): ContextRequirement[] {
  const owned = [chip.source, ...(chip.covers ?? [])];
  const others = requirements.filter((r) => !owned.includes(r));
  return requirements.includes(chip.source) ? others : [...others, ...owned];
}
