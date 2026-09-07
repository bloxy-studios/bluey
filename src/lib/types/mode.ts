/** Mode system types (mirrors `bluey_core::types::mode`). Modes are data, not code. */

export type PreferredLatency = "ultra-fast" | "fast" | "balanced" | "deep";

export type ContextRequirement =
  | "screen"
  | "accessibility"
  | "transcript"
  | "resume"
  | "job_description"
  | "documents"
  | "session_memory";

/** Which structured response shape the mode wants (see `response.ts`). */
export type ResponseSchemaId =
  | "answer"
  | "suggested-response"
  | "behavioral"
  | "coding"
  | "system-design"
  | "case"
  | "sales"
  | "recruiting"
  | "meeting"
  | "lecture";

export type ResponseLength = "concise" | "balanced" | "detailed";
export type ResponseTone = "natural" | "professional" | "technical" | "conversational" | "direct";

export interface ResponseStyle {
  length: ResponseLength;
  tone: ResponseTone;
}

export type ModelRole =
  | "default"
  | "fast"
  | "reasoning"
  | "vision"
  | "research"
  | "transcription"
  | "embedding";

export interface BlueyMode {
  id: string;
  name: string;
  description: string;
  /** Lucide icon name, e.g. "graduation-cap". */
  icon: string;
  systemInstructions: string;
  responseSchema: ResponseSchemaId;
  preferredLatency: PreferredLatency;
  contextRequirements: ContextRequirement[];
  /** Built-in modes cannot be deleted; their instructions can be edited. */
  builtIn: boolean;
  /** Sidebar group label, e.g. "Looking for work". */
  group?: string;
  /** Mode-level override for the user's global response style. */
  responseStyle?: Partial<ResponseStyle>;
  /** Which model role to prefer for the main answer. */
  preferredModelRole?: ModelRole;
  /** Documents attached at the mode level (job descriptions, notes...). */
  attachedDocumentIds: string[];
  createdAt: string;
  updatedAt: string;
}

export interface ModeDraft {
  name: string;
  description?: string;
  icon?: string;
  systemInstructions?: string;
  responseSchema?: ResponseSchemaId;
  preferredLatency?: PreferredLatency;
  contextRequirements?: ContextRequirement[];
  group?: string;
  responseStyle?: Partial<ResponseStyle>;
  preferredModelRole?: ModelRole;
}

export type ModePatch = Partial<ModeDraft>;

export const BUILT_IN_MODE_IDS = [
  "general",
  "interview",
  "behavioral-interview",
  "coding-interview",
  "system-design",
  "case-interview",
  "sales",
  "recruiting",
  "team-meeting",
  "lecture",
] as const;

export type BuiltInModeId = (typeof BUILT_IN_MODE_IDS)[number];
