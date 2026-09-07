import { buildPublicQuery, decideResearch, runResearch, OPTIMISTIC_AVAILABILITY } from "@/ai/research";
import type { RetrievedChunk, ScrapeResult, SearchResult } from "@/lib/types";
import { makeMode, makeSettings, makeSnapshot } from "../../fixtures/helpers/builders";

const NOW = () => new Date("2026-09-07T09:00:00.000Z");

const researchSettings = makeSettings({ ai: { researchEnabled: true, deepResearchEnabled: true } });

describe("decideResearch", () => {
  it("is none when research is disabled in settings", () => {
    expect(
      decideResearch({
        instruction: "latest news about Anthropic",
        mode: makeMode(),
        settings: makeSettings({ ai: { researchEnabled: false } }),
        availability: OPTIMISTIC_AVAILABILITY,
        now: NOW,
      }),
    ).toBe("none");
  });

  it("is none without an instruction", () => {
    expect(
      decideResearch({ mode: makeMode(), settings: researchSettings, availability: OPTIMISTIC_AVAILABILITY, now: NOW }),
    ).toBe("none");
  });

  it("never researches interview answers about the user's own experience", () => {
    const mode = makeMode({ id: "behavioral-interview", responseSchema: "behavioral" });
    expect(
      decideResearch({
        instruction: "help me talk about my experience leading migrations",
        mode,
        settings: researchSettings,
        availability: OPTIMISTIC_AVAILABILITY,
        now: NOW,
      }),
    ).toBe("none");
    expect(
      decideResearch({
        instruction: "tell me about a time you disagreed with your manager",
        mode,
        settings: researchSettings,
        availability: OPTIMISTIC_AVAILABILITY,
        now: NOW,
      }),
    ).toBe("none");
  });

  it("stays none in candidate modes without an explicit current-info ask", () => {
    const mode = makeMode({ id: "interview", responseSchema: "suggested-response" });
    expect(
      decideResearch({
        instruction: "who is Acme Corp",
        mode,
        settings: researchSettings,
        availability: OPTIMISTIC_AVAILABILITY,
        now: NOW,
      }),
    ).toBe("none");
    expect(
      decideResearch({
        instruction: "what is the latest Acme Corp funding news",
        mode,
        settings: researchSettings,
        availability: OPTIMISTIC_AVAILABILITY,
        now: NOW,
      }),
    ).toBe("search_scrape");
  });

  it("maps external-info cues to search_scrape and degrades with availability", () => {
    const args = {
      instruction: "what is the latest pricing for Vercel?",
      mode: makeMode(),
      settings: researchSettings,
      now: NOW,
    };
    expect(decideResearch({ ...args, availability: OPTIMISTIC_AVAILABILITY })).toBe("search_scrape");
    expect(decideResearch({ ...args, availability: { search: true, scrape: false, deepAgent: false } })).toBe("search");
    expect(decideResearch({ ...args, availability: { search: false, scrape: false, deepAgent: false } })).toBe("none");
  });

  it("uses deep_agent for deep-dive asks when enabled, else search_scrape", () => {
    const args = {
      instruction: "do a deep dive and compare vendors for feature flag platforms",
      mode: makeMode(),
      now: NOW,
    };
    expect(
      decideResearch({ ...args, settings: researchSettings, availability: OPTIMISTIC_AVAILABILITY }),
    ).toBe("deep_agent");
    expect(
      decideResearch({
        ...args,
        settings: makeSettings({ ai: { researchEnabled: true, deepResearchEnabled: false } }),
        availability: OPTIMISTIC_AVAILABILITY,
      }),
    ).toBe("search_scrape");
  });

  it("treats URLs and future years as external-info cues", () => {
    expect(
      decideResearch({
        instruction: "read https://example.com/changelog and tell me what changed",
        mode: makeMode(),
        settings: researchSettings,
        availability: OPTIMISTIC_AVAILABILITY,
        now: NOW,
      }),
    ).toBe("search_scrape");
    expect(
      decideResearch({
        instruction: "conference dates for KubeCon 2027",
        mode: makeMode(),
        settings: researchSettings,
        availability: OPTIMISTIC_AVAILABILITY,
        now: NOW,
      }),
    ).toBe("search_scrape");
  });
});

describe("buildPublicQuery", () => {
  it("strips emails, phone numbers and handles", () => {
    const query = buildPublicQuery(
      "what is the latest on Datadog pricing? reach me at jane.doe@example.com or +1 415 555 0199 or @janedoe",
    );
    expect(query).not.toContain("jane.doe@example.com");
    expect(query).not.toContain("415");
    expect(query).not.toContain("@janedoe");
    expect(query).toContain("Datadog");
  });

  it("strips the display name and resume proper nouns, keeps public entities", () => {
    const chunks: RetrievedChunk[] = [
      {
        chunkId: "c1",
        documentId: "d1",
        documentTitle: "Resume",
        documentKind: "resume",
        content: "Jane Doe led the payments team at HyperScale Inc for four years.",
        score: 0.9,
        scope: "global",
      },
    ];
    const snapshot = makeSnapshot({ userContext: { chunks, displayName: "Jane Doe" } });
    const query = buildPublicQuery("what is the latest Stripe news relevant to Jane Doe at HyperScale?", snapshot);
    expect(query).not.toMatch(/Jane|Doe|HyperScale/);
    expect(query).toContain("Stripe");
  });

  it("does not strip terms that only appear in public-ish documents", () => {
    const chunks: RetrievedChunk[] = [
      {
        chunkId: "c1",
        documentId: "d1",
        documentTitle: "JD",
        documentKind: "job_description",
        content: "Acme builds infrastructure for Kubernetes.",
        score: 0.8,
        scope: "mode",
      },
    ];
    const query = buildPublicQuery("latest Acme announcements?", makeSnapshot({ userContext: { chunks } }));
    expect(query).toContain("Acme");
  });

  it("caps the query length", () => {
    const query = buildPublicQuery(`research ${"word ".repeat(200)}`);
    expect(query.length).toBeLessThanOrEqual(300);
  });
});

describe("runResearch", () => {
  const results: SearchResult[] = [
    { id: "s1", title: "Result One", url: "https://one.test", snippet: "First snippet", source: "mock" },
    { id: "s2", title: "Result Two", url: "https://two.test", snippet: "Second snippet", source: "mock" },
    { id: "s3", title: "Result Three", url: "https://three.test", source: "mock" },
  ];

  function fakeResearchApi() {
    const scraped: string[] = [];
    return {
      scraped,
      api: {
        research: {
          search: async () => results,
          scrape: async ({ url }: { url: string }): Promise<ScrapeResult> => {
            scraped.push(url);
            return { url, title: `Page ${url}`, markdown: `# Content of ${url}\nBody text.`, source: "mock" };
          },
          deepStart: async () => undefined,
          deepCancel: async () => true,
        },
      },
    };
  }

  it("search depth returns citations and a context block without scraping", async () => {
    const { api, scraped } = fakeResearchApi();
    const outcome = await runResearch("search", "vercel pricing", { jobId: "res_1", api });
    expect(outcome?.citations.map((c) => c.url)).toEqual(["https://one.test", "https://two.test", "https://three.test"]);
    expect(outcome?.contextText).toContain("Result One");
    expect(outcome?.contextText).toContain("untrusted external content");
    expect(scraped).toEqual([]);
  });

  it("search_scrape scrapes the top results and includes page content", async () => {
    const { api, scraped } = fakeResearchApi();
    const outcome = await runResearch("search_scrape", "vercel pricing", { jobId: "res_2", api, scrapeTopN: 2 });
    expect(scraped).toEqual(["https://one.test", "https://two.test"]);
    expect(outcome?.contextText).toContain("Content of https://one.test");
  });

  it("returns null for depth none or an empty query, and on search failure", async () => {
    const { api } = fakeResearchApi();
    expect(await runResearch("none", "q", { jobId: "res_3", api })).toBeNull();
    expect(await runResearch("search", "  ", { jobId: "res_4", api })).toBeNull();
    const failing = {
      research: {
        ...fakeResearchApi().api.research,
        search: async (): Promise<SearchResult[]> => {
          throw new Error("no key");
        },
      },
    };
    expect(await runResearch("search", "q", { jobId: "res_5", api: failing })).toBeNull();
  });

  it("deep_agent resolves from research.event completion", async () => {
    const { api } = fakeResearchApi();
    type Handler = (payload: unknown) => void;
    const handlers = new Set<Handler>();
    const bus = {
      on: (_name: string, handler: Handler) => {
        handlers.add(handler);
        return () => handlers.delete(handler);
      },
      emit: (_name: string, payload: unknown) => {
        for (const handler of Array.from(handlers)) handler(payload);
      },
    };
    const promise = runResearch("deep_agent", "compare flag vendors", {
      jobId: "res_deep",
      api,
      bus: bus as never,
      timeoutMs: 5000,
    });
    bus.emit("research.event", {
      type: "completed",
      jobId: "res_deep",
      report: "Vendor A vs Vendor B report",
      citations: [{ id: "c1", title: "A", url: "https://a.test" }],
      totalMs: 1200,
      turns: 3,
    });
    const outcome = await promise;
    expect(outcome?.depth).toBe("deep_agent");
    expect(outcome?.contextText).toContain("Vendor A vs Vendor B report");
    expect(outcome?.citations).toHaveLength(1);
  });

  it("deep_agent resolves null on failure events", async () => {
    const { api } = fakeResearchApi();
    type Handler = (payload: unknown) => void;
    const handlers = new Set<Handler>();
    const bus = {
      on: (_name: string, handler: Handler) => {
        handlers.add(handler);
        return () => handlers.delete(handler);
      },
      emit: (_name: string, payload: unknown) => {
        for (const handler of Array.from(handlers)) handler(payload);
      },
    };
    const promise = runResearch("deep_agent", "q", { jobId: "res_fail", api, bus: bus as never, timeoutMs: 5000 });
    bus.emit("research.event", {
      type: "failed",
      jobId: "res_fail",
      error: { kind: "research", code: "research.failed", message: "x", recoverable: true },
    });
    expect(await promise).toBeNull();
  });
});
