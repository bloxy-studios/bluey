import { describe, expect, it } from "vitest";

import { outputLanguageLine } from "@/ai/prompts";
import { outputLanguageCode, outputLanguageName } from "@/lib/output-languages";

describe("output languages (UX-042)", () => {
  it("maps codes and legacy names both ways", () => {
    expect(outputLanguageCode("es")).toBe("es");
    expect(outputLanguageCode("Spanish")).toBe("es");
    expect(outputLanguageName("es")).toBe("Spanish");
    expect(outputLanguageName("it")).toBe("it");
  });

  it("asks the model for the language by name, and says nothing for English", () => {
    expect(outputLanguageLine("es")).toBe(
      "Respond in Spanish unless the user's question is written in another language.",
    );
    expect(outputLanguageLine("Spanish")).toBe(outputLanguageLine("es"));
    expect(outputLanguageLine("en")).toBe("");
    expect(outputLanguageLine("English")).toBe("");
    expect(outputLanguageLine("")).toBe("");
  });
});
