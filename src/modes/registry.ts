/**
 * Mode helpers: predicates and derived defaults over `BlueyMode` (modes are
 * data, not code) plus validation for user-created custom modes.
 */

import type {
  AITask,
  BlueyMode,
  ContextRequirement,
  ModeDraft,
  PreferredLatency,
  ResponseSchemaId,
  ResponseStyle,
  Settings,
} from "@/lib/types";

const CANDIDATE_MODE_IDS = new Set([
  "interview",
  "behavioral-interview",
  "coding-interview",
  "system-design",
  "case-interview",
]);

const CANDIDATE_SCHEMAS = new Set<ResponseSchemaId>([
  "suggested-response",
  "behavioral",
  "coding",
  "system-design",
  "case",
]);

/**
 * True when the user is the candidate (interview-side). Recruiting is the
 * other side of the table and is NOT a candidate mode.
 */
export function isCandidateMode(mode: BlueyMode): boolean {
  if (mode.id === "recruiting") return false;
  if (CANDIDATE_MODE_IDS.has(mode.id)) return true;
  if (mode.group?.toLowerCase().includes("looking for work")) return true;
  return CANDIDATE_SCHEMAS.has(mode.responseSchema);
}

export function requires(mode: BlueyMode, requirement: ContextRequirement): boolean {
  return mode.contextRequirements.includes(requirement);
}

/** Mode-level style override merged over the user's global response style. */
export function effectiveStyle(mode: BlueyMode, settings: Settings): ResponseStyle {
  return {
    length: mode.responseStyle?.length ?? settings.ai.responseLength,
    tone: mode.responseStyle?.tone ?? settings.ai.responseTone,
  };
}

/** Base AI task implied by the mode's response schema. */
export function defaultTaskFor(mode: BlueyMode): AITask {
  switch (mode.responseSchema) {
    case "coding":
      return "coding";
    case "system-design":
      return "system_design";
    default:
      return "answer";
  }
}

const VALID_SCHEMAS: readonly ResponseSchemaId[] = [
  "answer",
  "suggested-response",
  "behavioral",
  "coding",
  "system-design",
  "case",
  "sales",
  "recruiting",
  "meeting",
  "lecture",
];

const VALID_LATENCIES: readonly PreferredLatency[] = ["ultra-fast", "fast", "balanced", "deep"];

const VALID_REQUIREMENTS: readonly ContextRequirement[] = [
  "screen",
  "accessibility",
  "transcript",
  "resume",
  "job_description",
  "documents",
  "session_memory",
];

const VALID_LENGTHS = new Set(["concise", "balanced", "detailed"]);
const VALID_TONES = new Set(["natural", "professional", "technical", "conversational", "direct"]);

export interface ModeDraftValidation {
  ok: boolean;
  errors: string[];
}

/** Validate a custom mode draft before sending it to `modes_create`. */
export function validateModeDraft(draft: ModeDraft): ModeDraftValidation {
  const errors: string[] = [];

  const name = draft.name?.trim() ?? "";
  if (name.length === 0) errors.push("Name is required.");
  if (name.length > 60) errors.push("Name must be 60 characters or fewer.");

  if (draft.description !== undefined && draft.description.length > 300) {
    errors.push("Description must be 300 characters or fewer.");
  }
  if (draft.icon !== undefined && !/^[a-z0-9]+(-[a-z0-9]+)*$/.test(draft.icon)) {
    errors.push('Icon must be a lucide icon name in kebab-case (e.g. "graduation-cap").');
  }
  if (draft.systemInstructions !== undefined && draft.systemInstructions.length > 4000) {
    errors.push("System instructions must be 4000 characters or fewer.");
  }
  if (draft.responseSchema !== undefined && !VALID_SCHEMAS.includes(draft.responseSchema)) {
    errors.push(`Unknown response schema "${String(draft.responseSchema)}".`);
  }
  if (draft.preferredLatency !== undefined && !VALID_LATENCIES.includes(draft.preferredLatency)) {
    errors.push(`Unknown latency preference "${String(draft.preferredLatency)}".`);
  }
  for (const requirement of draft.contextRequirements ?? []) {
    if (!VALID_REQUIREMENTS.includes(requirement)) {
      errors.push(`Unknown context requirement "${String(requirement)}".`);
    }
  }
  if (draft.responseStyle?.length !== undefined && !VALID_LENGTHS.has(draft.responseStyle.length)) {
    errors.push(`Unknown response length "${String(draft.responseStyle.length)}".`);
  }
  if (draft.responseStyle?.tone !== undefined && !VALID_TONES.has(draft.responseStyle.tone)) {
    errors.push(`Unknown response tone "${String(draft.responseStyle.tone)}".`);
  }

  return { ok: errors.length === 0, errors };
}
