import { describe, expect, it } from "vitest";

import { buildSystemPrompt, PRIVACY_RULE } from "../../sidecars/agent/src/system-prompt";

describe("buildSystemPrompt", () => {
  const prompt = buildSystemPrompt({
    goal: "Determine the state of the art in on-device ASR",
    toolNames: ["exa_search", "firecrawl_scrape"],
    hasDocuments: false,
  });

  it("contains the privacy rule (public query, never private data)", () => {
    expect(prompt).toContain(PRIVACY_RULE);
    expect(PRIVACY_RULE).toMatch(/never request, infer/i);
    expect(PRIVACY_RULE).toMatch(/private/i);
  });

  it("instructs the agent to cite sources and never invent URLs", () => {
    expect(prompt).toMatch(/cite sources/i);
    expect(prompt).toMatch(/never invent/i);
  });

  it("instructs the agent to separate facts from inference", () => {
    expect(prompt).toMatch(/separate facts from inference/i);
  });

  it("embeds the research goal and the scoped tool names", () => {
    expect(prompt).toContain("Determine the state of the art in on-device ASR");
    expect(prompt).toContain("exa_search");
    expect(prompt).toContain("firecrawl_scrape");
    expect(prompt).not.toContain("document_read");
  });

  it("mentions document handling only when documents are allowed", () => {
    const withDocs = buildSystemPrompt({
      goal: "g",
      toolNames: ["document_read"],
      hasDocuments: true,
    });
    expect(withDocs).toMatch(/document_read/);
    expect(withDocs).toMatch(/never quote anything .* personal/i);
  });
});
