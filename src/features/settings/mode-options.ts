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

/** Context sources a mode gathers before answering. */
export const CONTEXT_SOURCE_LABELS: Record<ContextRequirement, string> = {
  screen: "Screen",
  accessibility: "Accessibility tree",
  transcript: "Transcript",
  resume: "Résumé / CV",
  job_description: "Job description",
  documents: "Documents",
  session_memory: "Session memory",
};

export const CONTEXT_SOURCES = Object.keys(CONTEXT_SOURCE_LABELS) as ContextRequirement[];
