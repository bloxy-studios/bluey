import { describe, expect, it } from "vitest";

import {
  ANTHROPIC_PRESET,
  GEMINI_PRESET,
  MODEL_ROLES,
  PROVIDER_PRESETS,
  applyPresets,
  presetForKind,
} from "@/lib/ai/provider-presets";
import type { ModelRoleAssignments } from "@/lib/types";

const EMPTY: ModelRoleAssignments = {
  default: null,
  fast: null,
  reasoning: null,
  vision: null,
  research: null,
  transcription: null,
  embedding: null,
};

describe("provider presets (TS mirror of bluey_core::presets)", () => {
  it("lists Gemini first and covers every real provider kind", () => {
    expect(PROVIDER_PRESETS[0]).toBe(GEMINI_PRESET);
    expect(PROVIDER_PRESETS.map((p) => p.kind)).toEqual([
      "google_gemini",
      "azure_foundry",
      "anthropic",
      "openai_compatible",
    ]);
    expect(PROVIDER_PRESETS.map((p) => p.id)).toEqual(["gemini", "azure-foundry", "anthropic", "openai"]);
    expect(presetForKind("mock")).toBeUndefined();
  });

  it("uses the verified Gemini model ids", () => {
    expect(GEMINI_PRESET.models).toEqual({
      default: "gemini-3.8-flash",
      fast: "gemini-3.5-flash-lite",
      reasoning: "gemini-3.8-flash",
      vision: "gemini-3.8-flash",
      research: "gemini-3.8-flash",
      transcription: "gemini-3.5-transcribe",
      embedding: "gemini-embedding-2",
    });
    expect(GEMINI_PRESET.requiresBaseUrl).toBe(false);
  });

  it("fills only empty roles unless overwriting", () => {
    const start: ModelRoleAssignments = {
      ...EMPTY,
      default: { providerId: "azure-foundry", model: "gpt-5.6-terra" },
    };
    const filled = applyPresets(start, { id: "gemini", kind: "google_gemini" }, false);
    expect(filled.changed).toHaveLength(6);
    expect(filled.models.default).toEqual({ providerId: "azure-foundry", model: "gpt-5.6-terra" });
    expect(filled.models.embedding).toEqual({ providerId: "gemini", model: "gemini-embedding-2" });

    const overwritten = applyPresets(filled.models, { id: "gemini", kind: "google_gemini" }, true);
    expect(overwritten.changed).toEqual(["default"]);
    expect(overwritten.models.default?.model).toBe("gemini-3.8-flash");

    expect(applyPresets(overwritten.models, { id: "gemini", kind: "google_gemini" }, true).changed).toEqual(
      [],
    );
  });

  it("skips roles a kind does not serve", () => {
    const result = applyPresets(EMPTY, { id: "anthropic", kind: "anthropic" }, false);
    expect(result.models.embedding).toBeNull();
    expect(result.models.transcription).toBeNull();
    expect(result.changed).toEqual(MODEL_ROLES.filter((r) => ANTHROPIC_PRESET.models[r] !== null));
    expect(() => applyPresets(EMPTY, { id: "mock", kind: "mock" }, false)).toThrow(/no preset/);
  });
});
