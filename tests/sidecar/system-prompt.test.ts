import { describe, expect, it } from "vitest";

import {
  buildSystemPrompt,
  PRIVACY_RULE,
  UNTRUSTED_TOOL_DATA_RULE,
} from "../../sidecars/agent/src/system-prompt";

describe("buildSystemPrompt", () => {
  const prompt = buildSystemPrompt({
    goal: "Determine the state of the art in on-device ASR",
    toolNames: ["exa_search", "firecrawl_scrape"],
    hasDocuments: false,
    maxTurns: 12,
    today: "2026-09-28",
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

  it("asks for a Sources section of the sources actually used (no garbled format line)", () => {
    expect(prompt).toContain("ending with a `## Sources` section that lists the sources you actually used");
    expect(prompt).not.toMatch(/intuition/);
  });

  it("states today's date, the turn budget and that tool output is untrusted", () => {
    expect(prompt).toContain("Today is 2026-09-28.");
    expect(prompt).toContain("at most 12 tool-calling turns");
    expect(prompt).toContain(UNTRUSTED_TOOL_DATA_RULE);
  });

  it("does not tell the agent to scrape when firecrawl_scrape is not available", () => {
    const searchOnly = buildSystemPrompt({
      goal: "g",
      toolNames: ["exa_search"],
      hasDocuments: false,
      maxTurns: 8,
      today: "2026-09-28",
    });
    expect(searchOnly).not.toContain("firecrawl_scrape");
  });

  it("mentions document handling only when documents are allowed", () => {
    const withDocs = buildSystemPrompt({
      goal: "g",
      toolNames: ["document_read"],
      hasDocuments: true,
      maxTurns: 12,
      today: "2026-09-28",
    });
    expect(withDocs).toMatch(/document_read/);
    expect(withDocs).toMatch(/never quote anything .* personal/i);
  });
});
