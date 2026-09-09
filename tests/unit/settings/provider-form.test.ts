import { describe, expect, it } from "vitest";

import {
  isPresetProviderId,
  newProviderFromDraft,
  providerBaseUrlPlaceholder,
  providerNeedsBaseUrl,
  providerToDraft,
  PROVIDER_KIND_OPTIONS,
  sortProviders,
} from "@/features/settings/provider-form";
import type { AIProviderConfig } from "@/lib/types";

const foundry: AIProviderConfig = {
  id: "azure-foundry",
  kind: "azure_foundry",
  name: "Azure Foundry",
  baseUrl: "https://res.openai.azure.com",
  enabled: true,
  hasApiKey: true,
};

describe("provider form helpers", () => {
  it("offers Gemini first and defaults new drafts to it", () => {
    expect(PROVIDER_KIND_OPTIONS[0]?.value).toBe("google_gemini");
    expect(providerToDraft().kind).toBe("google_gemini");
    expect(providerNeedsBaseUrl("google_gemini")).toBe(false);
    expect(providerNeedsBaseUrl("anthropic")).toBe(false);
    expect(providerNeedsBaseUrl("azure_foundry")).toBe(true);
    expect(providerNeedsBaseUrl("openai_compatible")).toBe(true);
  });

  it("uses the reserved id and preset name while they are free", () => {
    const created = newProviderFromDraft(
      { name: "", kind: "google_gemini", baseUrl: "", apiVersion: "", deployments: "" },
      [foundry],
    );
    expect(created).toMatchObject({ id: "gemini", name: "Google Gemini", kind: "google_gemini", baseUrl: "", enabled: true, hasApiKey: false });

    const second = newProviderFromDraft(
      { name: "Work Gemini", kind: "google_gemini", baseUrl: "https://proxy.example/v1beta", apiVersion: "", deployments: "" },
      [foundry, created],
    );
    expect(second.id).not.toBe("gemini");
    expect(second.name).toBe("Work Gemini");
    expect(second.baseUrl).toBe("https://proxy.example/v1beta");
  });

  it("keeps deployments only for Foundry", () => {
    const anthropic = newProviderFromDraft(
      { name: "", kind: "anthropic", baseUrl: "", apiVersion: "", deployments: "a=b" },
      [],
    );
    expect(anthropic.deployments).toBeUndefined();
    expect(anthropic.baseUrl).toBe("https://api.anthropic.com");
    const azure = newProviderFromDraft(
      { name: "", kind: "azure_foundry", baseUrl: "https://x.openai.azure.com", apiVersion: "preview", deployments: "gpt-6-astra=astra" },
      [],
    );
    expect(azure.deployments).toEqual({ "gpt-6-astra": "astra" });
    expect(azure.apiVersion).toBe("preview");
    expect(azure.id).toBe("azure-foundry");
  });

  it("sorts providers Gemini first, stable within a kind", () => {
    const gemini: AIProviderConfig = { ...foundry, id: "gemini", kind: "google_gemini", name: "Google Gemini" };
    const custom: AIProviderConfig = { ...foundry, id: "custom", kind: "openai_compatible", name: "Local" };
    expect(sortProviders([custom, foundry, gemini]).map((p) => p.id)).toEqual(["gemini", "azure-foundry", "custom"]);
  });

  it("recognises reserved ids and suggests the right endpoint per kind", () => {
    for (const id of ["gemini", "azure-foundry", "anthropic", "openai"]) expect(isPresetProviderId(id)).toBe(true);
    expect(isPresetProviderId("provider-abc123")).toBe(false);

    expect(providerBaseUrlPlaceholder("openai_compatible")).toBe("https://api.openai.com/v1");
    expect(providerBaseUrlPlaceholder("azure_foundry")).toBe("https://my-resource.openai.azure.com");
    expect(providerBaseUrlPlaceholder("google_gemini")).toBe("https://generativelanguage.googleapis.com/v1beta");
    expect(providerBaseUrlPlaceholder("anthropic")).toBe("https://api.anthropic.com");
  });
});
