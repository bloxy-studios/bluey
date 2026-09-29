import { describe, expect, it } from "vitest";

import { PromptBuilder } from "@/ai/prompt-builder";
import { createBuiltInModes } from "@/lib/tauri/mock";
import { BUILT_IN_MODE_IDS } from "@/lib/types";
import { validateModeDraft } from "@/modes/registry";
import { outputSchemaFor } from "@/modes/schemas";
import { rustBuiltInModes } from "../../fixtures/helpers/fixtures";

/**
 * The TS side works from the Rust built-in modes
 * (`tests/fixtures/rust/built-in-modes.json`, drift-checked by
 * `bluey-core/tests/ts_fixtures.rs`), never from a hand copy.
 */
describe("built-in modes parity with Rust", () => {
  const rust = rustBuiltInModes();

  it("lists the same ids as the TS constant, in seed order", () => {
    expect(rust.map((m) => m.id)).toEqual([...BUILT_IN_MODE_IDS]);
  });

  it("serves the Rust definitions from the mock transport", () => {
    const strip = ({ createdAt: _c, updatedAt: _u, ...rest }: (typeof rust)[number]) => rest;
    expect(createBuiltInModes().map(strip)).toEqual(rust.map(strip));
  });

  it.each(rust.map((m) => [m.id, m] as const))("%s fits the custom-mode limits", (_id, mode) => {
    expect(validateModeDraft(mode).errors).toEqual([]);
  });

  it.each(rust.map((m) => [m.id, m] as const))("%s renders its instructions into the system prompt", (_id, mode) => {
    const schemaId = mode.responseSchema;
    const system = new PromptBuilder({
      mode,
      style: { length: "concise", tone: "direct" },
      schemaId,
      trigger: "typed",
      items: [],
      instruction: "What should I say next?",
      outputSchema: outputSchemaFor(schemaId),
    }).renderSystem();
    expect(system).toContain(`Mode: ${mode.name}.`);
    expect(system).toContain(mode.systemInstructions);
  });
});
