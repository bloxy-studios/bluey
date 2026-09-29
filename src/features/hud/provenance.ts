import type { AIProviderConfig, BlueyResponse } from "@/lib/types";
import { formatMs } from "@/lib/utils/format";

export interface Provenance {
  /** "Google Gemini · gemini-2.5-flash · 1.40s". */
  text: string;
  /** "via API key" when a subscription account was skipped, else "fallback model". */
  marker?: string;
  /** The router's own explanation, for the marker's hover text. */
  detail?: string;
}

/**
 * Which provider and model answered, how long it took, and whether the router
 * fell back (UX-035). An account the router skipped (signed out, rate limited)
 * shows as "via API key": the answer came from a key-billed provider instead.
 */
export function describeProvenance(
  response: BlueyResponse,
  providers: readonly AIProviderConfig[],
  accountProviderIds: readonly string[],
): Provenance | null {
  const selection = response.selection;
  const model = selection?.model ?? response.metrics?.model;
  if (!model) return null;
  const providerId = selection?.providerId ?? response.metrics?.provider;
  const provider = providers.find((p) => p.id === providerId);
  const providerName = selection?.providerName ?? provider?.name;
  const totalMs = response.metrics?.totalMs;
  const text = [providerName, model, totalMs === undefined ? undefined : formatMs(totalMs)]
    .filter(Boolean)
    .join(" · ");

  const detail = selection?.fallbackReason;
  if (!detail) return { text };
  const answeredByAccount =
    provider?.authMethod === "oauth_subscription" || accountProviderIds.includes(providerId ?? "");
  const skippedAccount = accountProviderIds.some((id) => detail.includes(`provider ${id} `));
  const marker = skippedAccount && !answeredByAccount ? "via API key" : "fallback model";
  return { text, marker, detail };
}
