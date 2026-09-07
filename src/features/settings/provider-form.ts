import type { AIProviderConfig } from "@/lib/types";

export interface ProviderDraftValues {
  name: string;
  kind: AIProviderConfig["kind"];
  baseUrl: string;
  apiVersion: string;
  /** One `model=deployment` per line. */
  deployments: string;
}

export function providerToDraft(provider?: AIProviderConfig): ProviderDraftValues {
  return {
    name: provider?.name ?? "",
    kind: provider?.kind ?? "azure_foundry",
    baseUrl: provider?.baseUrl ?? "",
    apiVersion: provider?.apiVersion ?? "",
    deployments: provider?.deployments
      ? Object.entries(provider.deployments)
          .map(([model, deployment]) => `${model}=${deployment}`)
          .join("\n")
      : "",
  };
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
