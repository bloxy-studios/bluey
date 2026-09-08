/**
 * Provider presets — the TS mirror of `bluey_core::presets` (reserved ids,
 * display names and the recommended model per role). Used by the mock
 * transport and by Settings → AI to label "Use recommended models". The Rust
 * side is authoritative; keep both tables in sync.
 */

import type { AIProviderKind, ModelAssignment, ModelRoleAssignments } from "@/lib/types";
import type { ModelRole } from "@/lib/types";

export interface ProviderPreset {
  kind: AIProviderKind;
  /** Reserved provider id for this kind. */
  id: string;
  name: string;
  /** Default base URL ("" when the adapter knows the public endpoint or the user must supply one). */
  baseUrl: string;
  requiresBaseUrl: boolean;
  /** Recommended model per role (null = the kind does not serve the role). */
  models: Record<ModelRole, string | null>;
}

export const GEMINI_PRESET: ProviderPreset = {
  kind: "google_gemini",
  id: "gemini",
  name: "Google Gemini",
  baseUrl: "",
  requiresBaseUrl: false,
  models: {
    default: "gemini-3.8-flash",
    fast: "gemini-3.5-flash-lite",
    reasoning: "gemini-3.8-flash",
    vision: "gemini-3.8-flash",
    research: "gemini-3.8-flash",
    transcription: "gemini-3.5-transcribe",
    embedding: "gemini-embedding-2",
  },
};

export const AZURE_FOUNDRY_PRESET: ProviderPreset = {
  kind: "azure_foundry",
  id: "azure-foundry",
  name: "Microsoft Foundry",
  baseUrl: "",
  requiresBaseUrl: true,
  models: {
    default: "gpt-5.6-terra",
    fast: "gpt-5.6-luna",
    reasoning: "gpt-6-astra",
    vision: "gpt-6-astra",
    research: "gpt-6-astra",
    transcription: "MAI-Transcribe-1.5",
    embedding: "text-embedding-3-small",
  },
};

export const ANTHROPIC_PRESET: ProviderPreset = {
  kind: "anthropic",
  id: "anthropic",
  name: "Anthropic",
  baseUrl: "https://api.anthropic.com",
  requiresBaseUrl: false,
  models: {
    default: "claude-sonnet-5",
    fast: "claude-haiku-4-5",
    reasoning: "claude-opus-5",
    vision: "claude-sonnet-5",
    research: "claude-opus-5",
    transcription: null,
    embedding: null,
  },
};

export const OPENAI_PRESET: ProviderPreset = {
  kind: "openai_compatible",
  id: "openai",
  name: "OpenAI-compatible",
  baseUrl: "",
  requiresBaseUrl: true,
  models: {
    default: null,
    fast: null,
    reasoning: null,
    vision: null,
    research: null,
    transcription: null,
    embedding: null,
  },
};

/** Every preset in UI order (Gemini first). */
export const PROVIDER_PRESETS: readonly ProviderPreset[] = [
  GEMINI_PRESET,
  AZURE_FOUNDRY_PRESET,
  ANTHROPIC_PRESET,
  OPENAI_PRESET,
];

export const MODEL_ROLES: readonly ModelRole[] = [
  "default",
  "fast",
  "reasoning",
  "vision",
  "research",
  "transcription",
  "embedding",
];

export function presetForKind(kind: AIProviderKind): ProviderPreset | undefined {
  return PROVIDER_PRESETS.find((preset) => preset.kind === kind);
}

/**
 * Pure mirror of `bluey_core::presets::apply_presets`: returns the new
 * assignments and the roles that changed. `overwrite = false` fills only
 * unassigned roles.
 */
export function applyPresets(
  assignments: ModelRoleAssignments,
  provider: { id: string; kind: AIProviderKind },
  overwrite: boolean,
): { models: ModelRoleAssignments; changed: ModelRole[] } {
  const preset = presetForKind(provider.kind);
  if (!preset) throw new Error(`no preset for provider kind ${provider.kind}`);
  const models: ModelRoleAssignments = { ...assignments };
  const changed: ModelRole[] = [];
  for (const role of MODEL_ROLES) {
    const model = preset.models[role];
    if (!model) continue;
    if (!overwrite && assignments[role]) continue;
    const next: ModelAssignment = { providerId: provider.id, model };
    const current = assignments[role];
    if (current?.providerId === next.providerId && current.model === next.model) continue;
    models[role] = next;
    changed.push(role);
  }
  return { models, changed };
}
