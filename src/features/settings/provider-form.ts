import { presetForKind, PROVIDER_PRESETS } from "@/lib/ai/provider-presets";
import type { AIProviderConfig, AIProviderKind } from "@/lib/types";
import { createId } from "@/lib/utils/id";

export interface ProviderDraftValues {
  name: string;
  kind: AIProviderConfig["kind"];
  baseUrl: string;
  apiVersion: string;
  /** One `model=deployment` per line. */
  deployments: string;
}

/** Provider kinds offered in the dialog, Gemini first (ADR 0007). */
export const PROVIDER_KIND_OPTIONS: Array<{ value: AIProviderKind; label: string }> = [
  { value: "google_gemini", label: "Google Gemini (AI Studio)" },
  { value: "azure_foundry", label: "Microsoft Foundry / Azure OpenAI" },
  { value: "anthropic", label: "Anthropic" },
  { value: "openai_compatible", label: "OpenAI-compatible" },
];

export function providerKindLabel(kind: AIProviderKind): string {
  return PROVIDER_KIND_OPTIONS.find((option) => option.value === kind)?.label ?? kind;
}

/** Gemini and Anthropic have a fixed public endpoint; Foundry / OpenAI-compatible need one. */
export function providerNeedsBaseUrl(kind: AIProviderKind): boolean {
  return presetForKind(kind)?.requiresBaseUrl ?? kind !== "mock";
}

/** Where to get a key for this kind (shown next to the key field). */
export function providerKeyHelp(kind: AIProviderKind): { label: string; url: string } | null {
  switch (kind) {
    case "google_gemini":
      return {
        label: "Get a free key at aistudio.google.com/apikey",
        url: "https://aistudio.google.com/apikey",
      };
    case "anthropic":
      return {
        label: "Keys live in console.anthropic.com",
        url: "https://console.anthropic.com/settings/keys",
      };
    default:
      return null;
  }
}

export function providerToDraft(provider?: AIProviderConfig): ProviderDraftValues {
  return {
    name: provider?.name ?? "",
    kind: provider?.kind ?? "google_gemini",
    baseUrl: provider?.baseUrl ?? "",
    apiVersion: provider?.apiVersion ?? "",
    deployments: provider?.deployments
      ? Object.entries(provider.deployments)
          .map(([model, deployment]) => `${model}=${deployment}`)
          .join("\n")
      : "",
  };
}

/** Default name / base URL for a kind when the user has not typed one. */
export function draftDefaultsFor(kind: AIProviderKind): { name: string; baseUrl: string } {
  const preset = presetForKind(kind);
  return { name: preset?.name ?? "", baseUrl: preset?.baseUrl ?? "" };
}

export function draftToDeployments(text: string): Record<string, string> | undefined {
  const entries = text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.includes("="))
    .map((line) => {
      const index = line.indexOf("=");
      return [line.slice(0, index).trim(), line.slice(index + 1).trim()] as const;
    })
    .filter(([model, deployment]) => model.length > 0 && deployment.length > 0);
  return entries.length > 0 ? Object.fromEntries(entries) : undefined;
}

/**
 * Build a new provider from the dialog values. The kind's reserved id
 * (`gemini`, `azure-foundry`, …) is used when it is still free so `.env`
 * imports, presets and the UI all agree on the same provider.
 */
export function newProviderFromDraft(
  values: ProviderDraftValues,
  existing: AIProviderConfig[],
): AIProviderConfig {
  const preset = presetForKind(values.kind);
  const reservedFree = preset !== undefined && !existing.some((p) => p.id === preset.id);
  const defaults = draftDefaultsFor(values.kind);
  return {
    id: reservedFree ? preset.id : createId("provider"),
    kind: values.kind,
    name: values.name.trim() || defaults.name || "Provider",
    baseUrl: values.baseUrl.trim() || defaults.baseUrl,
    apiVersion: values.apiVersion.trim() || undefined,
    deployments: values.kind === "azure_foundry" ? draftToDeployments(values.deployments) : undefined,
    enabled: true,
    hasApiKey: false,
  };
}

/** Providers in UI order: preset kinds (Gemini first) then everything else, stable within a kind. */
export function sortProviders(providers: AIProviderConfig[]): AIProviderConfig[] {
  const rank = (kind: AIProviderKind) => {
    const index = PROVIDER_PRESETS.findIndex((preset) => preset.kind === kind);
    return index === -1 ? PROVIDER_PRESETS.length : index;
  };
  return [...providers].sort((a, b) => rank(a.kind) - rank(b.kind));
}
