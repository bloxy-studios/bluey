import { maxOutputTokensFor, STRUCTURED_OUTPUT_OVERHEAD_TOKENS } from "@/ai/request";

describe("maxOutputTokensFor", () => {
  it("never drops below the length and task budgets", () => {
    expect(maxOutputTokensFor("concise", "answer")).toBe(600);
    expect(maxOutputTokensFor("balanced", "answer")).toBe(1200);
    expect(maxOutputTokensFor("detailed", "answer")).toBe(2400);
    expect(maxOutputTokensFor("concise", "coding")).toBe(1600);
    expect(maxOutputTokensFor("concise", "system_design")).toBe(2400);
    expect(maxOutputTokensFor("concise", "summarization")).toBe(1600);
  });

  it("raises the budget to the shape's floor", () => {
    expect(maxOutputTokensFor("concise", "answer", "spoken")).toBe(700);
    expect(maxOutputTokensFor("concise", "answer", "written")).toBe(700);
    expect(maxOutputTokensFor("concise", "coding", "code")).toBe(2000);
    expect(maxOutputTokensFor("concise", "system_design", "design")).toBe(3000);
    // A pick needs less than the concise length — the length still wins.
    expect(maxOutputTokensFor("concise", "answer", "choice")).toBe(600);
    expect(maxOutputTokensFor("detailed", "answer", "boolean")).toBe(2400);
  });

  it("adds the structured-output overhead when a schema is sent", () => {
    expect(maxOutputTokensFor("concise", "answer", "choice", true)).toBe(600 + STRUCTURED_OUTPUT_OVERHEAD_TOKENS);
    expect(maxOutputTokensFor("concise", "coding", "code", true)).toBe(2000 + STRUCTURED_OUTPUT_OVERHEAD_TOKENS);
    expect(maxOutputTokensFor("concise", "answer", "choice", false)).toBe(600);
  });
});
