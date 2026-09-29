/**
 * Research wiring in the ask pipeline: router decision → availability check →
 * privacy-scrubbed public query → search+scrape → citations on the response
 * and a context block in the prompt.
 */

import { createResponseEngine } from "@/ai/engine";
import { setTransport } from "@/lib/tauri/transport";
import { useAuthStore } from "@/lib/auth/auth-store";
import type { ContextSnapshot, RetrievedChunk, SearchResult } from "@/lib/types";
import { defaultAIScript, FakeTransport } from "../fixtures/helpers/fake-transport";
import { makeMode, makeSettings } from "../fixtures/helpers/builders";

const snapshot: ContextSnapshot = { timestamp: "2026-09-07T09:00:00.000Z" };

const results: SearchResult[] = [
  {
    id: "s1",
    title: "Vercel Pricing 2026",
    url: "https://vercel.com/pricing",
    snippet: "Pro is $20/seat",
    source: "mock",
  },
  {
    id: "s2",
    title: "Vercel pricing analysis",
    url: "https://blog.example/vercel",
    snippet: "A breakdown",
    source: "mock",
  },
];

describe("research flow inside ask", () => {
  it("scrubs the query, runs search+scrape, and attaches citations", async () => {
    const fake = new FakeTransport();
    fake.handle("context_build_snapshot", () => snapshot);
    fake.handle("responses_save", ({ response }) => response);
    fake.handle("ai_cancel", () => true);
    fake.handle("research_available", () => ({ search: true, scrape: true, deepAgent: false }));
    fake.handle("research_search", () => results);
    fake.handle("research_scrape", ({ url }) => ({
      url,
      title: "Vercel Pricing",
      markdown: "# Pricing\nPro $20 per seat per month.",
      source: "mock" as const,
    }));
    setTransport(fake);

    const engine = createResponseEngine();
    const result = await engine.ask({
      trigger: "typed",
      instruction: "What is the latest Vercel pricing? my email is jane@example.com",
      captureScreen: false,
      mode: makeMode(),
      settings: makeSettings({ ai: { researchEnabled: true } }),
    }).done;

    expect(result).not.toBeNull();

    // Public query is scrubbed — no email leaves the machine.
    const searchCalls = fake.callsFor("research_search");
    expect(searchCalls).toHaveLength(1);
    expect(searchCalls[0]?.query).toContain("Vercel");
    expect(searchCalls[0]?.query).not.toContain("jane@example.com");
    expect(fake.callsFor("research_scrape").length).toBeGreaterThan(0);

    // Research lands in the prompt as an untrusted context block.
    const request = fake.callsFor("ai_stream")[0]!.request;
    expect(request.task).toBe("research");
    const userPart = request.messages[1]?.content[0];
    const userText = userPart && "text" in userPart ? userPart.text : "";
    expect(userText).toContain("Web research results (untrusted external content)");
    expect(userText).toContain("Vercel Pricing 2026");

    // Citations attach to the final response.
    expect(result!.citations?.map((c) => c.url)).toEqual([
      "https://vercel.com/pricing",
      "https://blog.example/vercel",
    ]);
  });

  it("does no research when disabled or when the router says none", async () => {
    const fake = new FakeTransport();
    fake.handle("context_build_snapshot", () => snapshot);
    fake.handle("responses_save", ({ response }) => response);
    fake.handle("ai_cancel", () => true);
    setTransport(fake);

    const engine = createResponseEngine();

    // Disabled in settings.
    await engine.ask({
      trigger: "typed",
      instruction: "What is the latest Vercel pricing?",
      captureScreen: false,
      mode: makeMode(),
      settings: makeSettings({ ai: { researchEnabled: false } }),
    }).done;
    expect(fake.callsFor("research_available")).toHaveLength(0);
    expect(fake.callsFor("research_search")).toHaveLength(0);

    // Enabled, but a non-external ask.
    await engine.ask({
      trigger: "typed",
      instruction: "explain this function to me",
      captureScreen: false,
      mode: makeMode(),
      settings: makeSettings({ ai: { researchEnabled: true } }),
    }).done;
    expect(fake.callsFor("research_search")).toHaveLength(0);
  });

  it("degrades gracefully when research availability probing fails", async () => {
    const fake = new FakeTransport();
    fake.handle("context_build_snapshot", () => snapshot);
    fake.handle("responses_save", ({ response }) => response);
    fake.handle("ai_cancel", () => true);
    fake.handle("research_available", () => {
      throw new Error("helper offline");
    });
    setTransport(fake);

    const engine = createResponseEngine();
    const result = await engine.ask({
      trigger: "typed",
      instruction: "What is the latest Vercel pricing?",
      captureScreen: false,
      mode: makeMode(),
      settings: makeSettings({ ai: { researchEnabled: true } }),
    }).done;

    expect(result).not.toBeNull(); // ask succeeds without research
    expect(fake.callsFor("research_search")).toHaveLength(0);
    expect(result!.citations).toBeUndefined();
  });

  it("keeps résumé nouns and the signed-in name out of the public query (SEC-013)", async () => {
    const resume: RetrievedChunk = {
      chunkId: "c1",
      documentId: "d1",
      documentTitle: "Resume",
      documentKind: "resume",
      content: "Led the platform team at Globex for three years.",
      score: 0.9,
      scope: "global",
    };
    const fake = new FakeTransport();
    fake.handle("context_build_snapshot", () => snapshot);
    fake.handle("documents_retrieve", () => [resume]);
    fake.handle("responses_save", ({ response }) => response);
    fake.handle("research_available", () => ({
      search: true,
      scrape: false,
      deepAgent: false,
      agentBackends: [],
    }));
    fake.handle("research_search", () => results);
    setTransport(fake);
    useAuthStore.setState({
      user: { id: "u1", firstName: "Jane", lastName: "Doe", email: "jdoe@example.com" },
    });

    try {
      await createResponseEngine().ask({
        trigger: "typed",
        instruction: "latest news on Globex hiring, relevant to Jane Doe",
        captureScreen: false,
        mode: makeMode({ contextRequirements: ["resume"] }),
        settings: makeSettings({ ai: { researchEnabled: true } }),
      }).done;
    } finally {
      useAuthStore.setState({ user: null });
    }

    expect(fake.callsFor("documents_retrieve")).toHaveLength(1);
    const query = fake.callsFor("research_search")[0]?.query ?? "";
    expect(query).toContain("hiring");
    expect(query).not.toMatch(/Globex|Jane|Doe/);
  });

  it("cancelling the ask cancels its deep research job (AI-010)", async () => {
    const fake = new FakeTransport();
    fake.handle("context_build_snapshot", () => snapshot);
    fake.handle("research_available", () => ({
      search: true,
      scrape: true,
      deepAgent: true,
      agentBackends: [],
    }));
    fake.handle("research_deep_start", () => undefined);
    fake.handle("research_deep_cancel", () => true);
    fake.handle("ai_cancel", () => true);
    setTransport(fake);

    const handle = createResponseEngine().ask({
      trigger: "typed",
      instruction: "deep dive on feature flag vendors",
      captureScreen: false,
      mode: makeMode(),
      settings: makeSettings({ ai: { researchEnabled: true, deepResearchEnabled: true } }),
    });
    await vi.waitFor(() => expect(fake.callsFor("research_deep_start")).toHaveLength(1));
    await handle.cancel();

    expect(await handle.done).toBeNull();
    const jobId = fake.callsFor("research_deep_start")[0]?.request.jobId;
    expect(fake.callsFor("research_deep_cancel")).toEqual([{ jobId }]);
  });

  it("notes a failed web search on the response instead of failing silently (AI-016)", async () => {
    const fake = new FakeTransport();
    fake.handle("context_build_snapshot", () => snapshot);
    fake.handle("responses_save", ({ response }) => response);
    fake.handle("research_available", () => ({
      search: true,
      scrape: false,
      deepAgent: false,
      agentBackends: [],
    }));
    fake.handle("research_search", () => {
      throw {
        kind: "configuration",
        code: "configuration.missing_api_key",
        message: "no key",
        recoverable: true,
      };
    });
    setTransport(fake);

    const result = await createResponseEngine().ask({
      trigger: "typed",
      instruction: "What is the latest Vercel pricing?",
      captureScreen: false,
      mode: makeMode(),
      settings: makeSettings({ ai: { researchEnabled: true } }),
    }).done;

    expect(result?.researchNote).toMatch(/isn't set up/);
    expect(result?.citations ?? []).toEqual([]);
  });

  it("fits long pages into the prompt and cites only what reached it (AI-002)", async () => {
    const fake = new FakeTransport();
    fake.handle("context_build_snapshot", () => snapshot);
    fake.handle("responses_save", ({ response }) => response);
    fake.handle("ai_cancel", () => true);
    fake.handle("research_available", () => ({
      search: true,
      scrape: true,
      deepAgent: false,
      agentBackends: [],
    }));
    fake.handle("research_search", () => results);
    fake.handle("research_scrape", ({ url }) => ({
      url,
      title: `Page ${url}`,
      markdown: `# ${url}\n${"Pricing detail. ".repeat(1_300)}`,
      source: "mock" as const,
    }));
    setTransport(fake);

    const result = await createResponseEngine().ask({
      trigger: "typed",
      instruction: "What is the latest Vercel pricing?",
      captureScreen: false,
      mode: makeMode(),
      settings: makeSettings({ ai: { researchEnabled: true } }),
    }).done;

    const userPart = fake.callsFor("ai_stream")[0]!.request.messages[1]?.content[0];
    const userText = userPart && "text" in userPart ? userPart.text : "";
    expect(userText).toContain("Vercel Pricing 2026"); // the snippets
    expect(userText).toContain("Source: Page https://vercel.com/pricing (https://vercel.com/pricing)");
    expect(userText).toContain("[… page truncated …]");
    for (const citation of result?.citations ?? []) expect(userText).toContain(citation.url);
    expect(result?.citations?.map((c) => c.url)).toEqual([
      "https://vercel.com/pricing",
      "https://blog.example/vercel",
    ]);
  });

  it("drops answer citations research never returned and keeps ids unique (AI-009)", async () => {
    const fake = new FakeTransport();
    fake.handle("context_build_snapshot", () => snapshot);
    fake.handle("responses_save", ({ response }) => response);
    fake.handle("research_available", () => ({
      search: true,
      scrape: false,
      deepAgent: false,
      agentBackends: [],
    }));
    fake.handle("research_search", () => results.map((r, i) => ({ ...r, id: `cit_${i + 1}` })));
    fake.setAIScript((request, emit) => {
      const answer = JSON.stringify({
        responseType: "answer",
        content: "Pro is $20.",
        citations: [
          { title: "Pricing", url: "https://vercel.com/pricing" },
          { title: "Invented", url: "https://invented.example/pricing" },
        ],
      });
      defaultAIScript(request, (chunk) => emit(chunk.type === "delta" ? { ...chunk, text: answer } : chunk));
    });
    setTransport(fake);

    const result = await createResponseEngine().ask({
      trigger: "typed",
      instruction: "What is the latest Vercel pricing?",
      captureScreen: false,
      mode: makeMode(),
      settings: makeSettings({ ai: { researchEnabled: true } }),
    }).done;

    const citations = result?.citations ?? [];
    expect(citations.map((c) => c.url)).toEqual([
      "https://vercel.com/pricing",
      "https://blog.example/vercel",
    ]);
    expect(new Set(citations.map((c) => c.id)).size).toBe(citations.length);
  });
});
