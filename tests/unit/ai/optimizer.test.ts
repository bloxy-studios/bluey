import { deriveTitle, firstCodeBlock, optimizeResponse, stripFillerOpeners } from "@/ai/optimizer";
import type { BlueyResponse, ResponseStyle } from "@/lib/types";
import { makeMode } from "../../fixtures/helpers/builders";

const STYLE_BALANCED: ResponseStyle = { length: "balanced", tone: "natural" };
const STYLE_CONCISE: ResponseStyle = { length: "concise", tone: "natural" };

function response(content: string, overrides: Partial<BlueyResponse> = {}): BlueyResponse {
  return {
    id: "resp_1",
    requestId: "req_1",
    modeId: "general",
    type: "answer",
    content,
    createdAt: "2026-09-07T09:00:00.000Z",
    ...overrides,
  };
}

describe("optimizeResponse", () => {
  it("strips filler openers", () => {
    const optimized = optimizeResponse(
      response("Certainly! Great question! the index is missing on the email column."),
      { style: STYLE_BALANCED, mode: makeMode() },
    );
    expect(optimized.content.startsWith("The index is missing")).toBe(true);
  });

  it("removes repeated paragraphs and collapses extra blank lines", () => {
    const optimized = optimizeResponse(
      response("Add an index.\n\n\n\nAdd an index.\n\nThen re-run the query."),
      { style: STYLE_BALANCED, mode: makeMode() },
    );
    expect(optimized.content).toBe("Add an index.\n\nThen re-run the query.");
  });

  it("preserves code blocks verbatim, even under the concise cap", () => {
    const code = "```python\ndef two_sum(nums, target):\n    seen = {}\n    for i, n in enumerate(nums):\n        if target - n in seen:\n            return [seen[target - n], i]\n        seen[n] = i\n```";
    const longProse = Array.from({ length: 60 }, (_, i) => `Sentence number ${i} explains a detail.`).join(" ");
    const optimized = optimizeResponse(response(`Use a hash map.\n\n${code}\n\n${longProse}`), {
      style: STYLE_CONCISE,
      mode: makeMode({ responseSchema: "coding" }),
    });
    expect(optimized.content).toContain(code);
  });

  it("caps concise prose at roughly 120 words when there is no code", () => {
    const paragraphs = Array.from({ length: 12 }, (_, i) =>
      `Paragraph ${i} has exactly nine words in this sentence okay.`,
    ).join("\n\n");
    const optimized = optimizeResponse(response(paragraphs), { style: STYLE_CONCISE, mode: makeMode() });
    const words = optimized.content.split(/\s+/).filter((w) => w.length > 0).length;
    expect(words).toBeLessThanOrEqual(135);
  });

  it("keeps caveat and citation paragraphs past the length cap", () => {
    const filler = Array.from({ length: 20 }, (_, i) =>
      `Filler paragraph ${i} adds ten words to the total count now.`,
    ).join("\n\n");
    const caveat = "Note: this pricing may be outdated — double-check the vendor page.";
    const citation = "Source: https://vendor.example/pricing [1]";
    const optimized = optimizeResponse(response(`${filler}\n\n${caveat}\n\n${citation}`), {
      style: STYLE_CONCISE,
      mode: makeMode(),
    });
    expect(optimized.content).toContain(caveat);
    expect(optimized.content).toContain(citation);
  });

  it("derives a title from the first heading, else from the prompt", () => {
    const fromHeading = optimizeResponse(response("## Fix the N+1 query\n\nBatch the loads."), {
      style: STYLE_BALANCED,
      mode: makeMode(),
    });
    expect(fromHeading.title).toBe("Fix the N+1 query");

    const fromPrompt = optimizeResponse(response("Batch the loads.", { prompt: "Why is my page slow?" }), {
      style: STYLE_BALANCED,
      mode: makeMode(),
    });
    expect(fromPrompt.title).toBe("Why is my page slow?");
  });

  it("keeps an existing title", () => {
    const optimized = optimizeResponse(response("Body.", { title: "Kept" }), {
      style: STYLE_BALANCED,
      mode: makeMode(),
    });
    expect(optimized.title).toBe("Kept");
  });

  it("populates code from the first fenced block for coding responses", () => {
    const optimized = optimizeResponse(
      response("Approach first.\n\n```go\nfunc add(a, b int) int { return a + b }\n```", { type: "code" }),
      { style: STYLE_BALANCED, mode: makeMode({ responseSchema: "coding" }) },
    );
    expect(optimized.code).toEqual({ language: "go", code: "func add(a, b int) int { return a + b }" });
  });

  it("keeps sections and citations untouched", () => {
    const optimized = optimizeResponse(
      response("Body.", {
        sections: [{ id: "sec_1", title: "Approach", content: "Hash map." }],
        citations: [{ id: "cit_1", title: "Docs", url: "https://docs.example" }],
      }),
      { style: STYLE_BALANCED, mode: makeMode() },
    );
    expect(optimized.sections).toHaveLength(1);
    expect(optimized.citations).toHaveLength(1);
  });
});

describe("helpers", () => {
  it("stripFillerOpeners removes stacked openers and re-capitalizes", () => {
    expect(stripFillerOpeners("Sure! Of course! here is the fix: use a mutex.")).toBe("Use a mutex.");
    expect(stripFillerOpeners("No filler here.")).toBe("No filler here.");
  });

  it("firstCodeBlock parses language and body", () => {
    expect(firstCodeBlock("pre\n```rust\nfn main() {}\n```\npost")).toEqual({
      language: "rust",
      code: "fn main() {}",
    });
    expect(firstCodeBlock("no code")).toBeNull();
  });

  it("deriveTitle truncates long prompts", () => {
    const title = deriveTitle("body", "w".repeat(80));
    expect(title?.length).toBeLessThanOrEqual(60);
    expect(title?.endsWith("…")).toBe(true);
  });
});
