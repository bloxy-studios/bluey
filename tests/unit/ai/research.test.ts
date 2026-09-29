import {
  buildPublicQuery,
  decideResearch,
  keptResearchCitations,
  runResearch,
  skipLocalResearch,
  OPTIMISTIC_AVAILABILITY,
} from "@/ai/research";
import type { DeepResearchEvent, DeepResearchRequest } from "@/lib/types";
import type { RetrievedChunk, ScrapeResult, SearchResult } from "@/lib/types";
import { makeMode, makeSettings } from "../../fixtures/helpers/builders";

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
      decideResearch({
        mode: makeMode(),
        settings: researchSettings,
        availability: OPTIMISTIC_AVAILABILITY,
        now: NOW,
      }),
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
    expect(decideResearch({ ...args, availability: { search: true, scrape: false, deepAgent: false } })).toBe(
      "search",
    );
    expect(
      decideResearch({ ...args, availability: { search: false, scrape: false, deepAgent: false } }),
    ).toBe("none");
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
    const query = buildPublicQuery("what is the latest Stripe news relevant to Jane Doe at HyperScale?", {
      chunks,
      names: ["Jane Doe"],
    });
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
    const query = buildPublicQuery("latest Acme announcements?", { chunks });
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
    expect(outcome?.citations.map((c) => c.url)).toEqual([
      "https://one.test",
      "https://two.test",
      "https://three.test",
    ]);
    expect(outcome?.items.map((i) => i.ref)).toEqual(["research:snippets"]);
    expect(outcome?.items[0]?.content).toContain("Result One");
    expect(outcome?.items[0]?.content).toContain("untrusted external content");
    expect(scraped).toEqual([]);
  });

  it("search_scrape scrapes the top results and includes page content", async () => {
    const { api, scraped } = fakeResearchApi();
    const outcome = await runResearch("search_scrape", "vercel pricing", {
      jobId: "res_2",
      api,
      scrapeTopN: 2,
    });
    expect(scraped).toEqual(["https://one.test", "https://two.test"]);
    expect(outcome?.items.map((i) => i.ref)).toEqual([
      "research:snippets",
      "research:page:1",
      "research:page:2",
    ]);
    expect(outcome?.items[1]?.content).toContain("Source: Page https://one.test (https://one.test)");
    expect(outcome?.items[1]?.content).toContain("Content of https://one.test");
  });

  it("returns null for depth none or an empty query, and a note on search failure", async () => {
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
    const failed = await runResearch("search", "q", { jobId: "res_5", api: failing });
    expect(failed?.items).toEqual([]);
    expect(failed?.citations).toEqual([]);
    expect(failed?.note).toMatch(/Web research failed/);
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
    expect(outcome?.items[0]?.ref).toBe("research:report");
    expect(outcome?.items[0]?.content).toContain("Vendor A vs Vendor B report");
    expect(outcome?.citations).toHaveLength(1);
  });

  it("deep_agent turns a failure event into a note", async () => {
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
    const promise = runResearch("deep_agent", "q", {
      jobId: "res_fail",
      api,
      bus: bus as never,
      timeoutMs: 5000,
    });
    bus.emit("research.event", {
      type: "failed",
      jobId: "res_fail",
      error: {
        kind: "configuration",
        code: "configuration.missing_api_key",
        message: "x",
        recoverable: true,
      },
    });
    expect((await promise)?.note).toMatch(/isn't set up/);
  });
});

describe("research cues (LIVE-006)", () => {
  const decide = (instruction: string) =>
    decideResearch({
      instruction,
      mode: makeMode(),
      settings: researchSettings,
      availability: OPTIMISTIC_AVAILABILITY,
      now: NOW,
    });

  it("does not search for a bare 'current' or 'recent'", () => {
    expect(decide("fix the current function")).toBe("none");
    expect(decide("explain the recent change in this diff")).toBe("none");
  });

  it("still searches when 'current' sits next to a time-sensitive noun", () => {
    expect(decide("what is the current price of bitcoin")).not.toBe("none");
  });
});

const hits: SearchResult[] = [
  { id: "h1", title: "One", url: "https://one.test", snippet: "s1", source: "mock" },
  { id: "h2", title: "Two", url: "https://two.test", snippet: "s2", source: "mock" },
  { id: "h3", title: "Three", url: "https://three.test", source: "mock" },
];

function slowApi(scrape: (url: string) => Promise<ScrapeResult>) {
  const cancelled: string[] = [];
  const started: DeepResearchRequest[] = [];
  return {
    cancelled,
    started,
    api: {
      research: {
        search: async () => hits,
        scrape: ({ url }: { url: string }) => scrape(url),
        deepStart: async ({ request }: { request: DeepResearchRequest }) => {
          started.push(request);
        },
        deepCancel: async ({ jobId }: { jobId: string }) => {
          cancelled.push(jobId);
          return true;
        },
      },
    },
  };
}

const page = (url: string, markdown = `Body of ${url}`): ScrapeResult => ({
  url,
  title: `Page ${url}`,
  markdown,
  source: "mock",
});
const delay = <T>(ms: number, value: T) => new Promise<T>((resolve) => setTimeout(() => resolve(value), ms));

describe("search_scrape budget and deadlines (AI-002, LIVE-006)", () => {
  it("caps each page and gives it a Source header", async () => {
    const { api } = slowApi(async (url) => page(url, "x".repeat(20_000)));
    const outcome = await runResearch("search_scrape", "q", { jobId: "res_cap", api, scrapeTopN: 1 });
    const item = outcome!.items[1]!;
    expect(item.content).toContain("Source: Page https://one.test (https://one.test)");
    expect(item.content).toContain("[… page truncated …]");
    expect(item.content.length).toBeLessThan(6_300);
  });

  it("scrapes the top results in parallel", async () => {
    const { api } = slowApi((url) => delay(150, page(url)));
    const started = Date.now();
    const outcome = await runResearch("search_scrape", "q", { jobId: "res_par", api });
    expect(outcome?.items).toHaveLength(4);
    expect(Date.now() - started).toBeLessThan(400);
  });

  it("keeps the snippets when a page never loads before the search deadline", async () => {
    const { api } = slowApi(() => new Promise<ScrapeResult>(() => {}));
    const outcome = await runResearch("search_scrape", "q", { jobId: "res_hang", api, searchTimeoutMs: 100 });
    expect(outcome?.items.map((i) => i.ref)).toEqual(["research:snippets"]);
    expect(outcome?.note).toBeUndefined();
  });

  it("shows only sources whose item reached the prompt", () => {
    const outcome = {
      depth: "search_scrape" as const,
      items: [
        {
          ref: "research:snippets",
          content: "",
          relevance: 0.8,
          citations: [{ id: "a", title: "A", url: "https://a.test" }],
        },
        {
          ref: "research:page:1",
          content: "",
          relevance: 0.7,
          citations: [{ id: "b", title: "B", url: "https://b.test" }],
        },
      ],
      citations: [],
    };
    const kept = keptResearchCitations(outcome, new Set(["research:page:1"]));
    expect(kept.citations.map((c) => c.url)).toEqual(["https://b.test"]);
  });
});

describe("research status and cancellation (LIVE-006, AI-010, PROV-013)", () => {
  function recordingBus() {
    const events: DeepResearchEvent[] = [];
    const handlers = new Set<(e: DeepResearchEvent) => void>();
    return {
      events,
      bus: {
        on: (_name: "research.event", handler: (e: DeepResearchEvent) => void) => {
          handlers.add(handler);
          return () => void handlers.delete(handler);
        },
        emit: (_name: "research.event", event: DeepResearchEvent) => {
          events.push(event);
          for (const handler of Array.from(handlers)) handler(event);
        },
      },
    };
  }

  it("publishes the search path's progress on the research channel", async () => {
    const { api } = slowApi(async (url) => page(url));
    const { bus, events } = recordingBus();
    await runResearch("search_scrape", "q", { jobId: "res_status", api, bus });
    expect(events.map((e) => (e.type === "tool_call" ? e.tool : e.type))).toEqual([
      "started",
      "exa_search",
      "firecrawl_scrape",
      "completed",
    ]);
  });

  it("Skip ends a search job at once and the ask continues without research", async () => {
    const { api } = slowApi(() => new Promise<ScrapeResult>(() => {}));
    const pending = runResearch("search_scrape", "q", { jobId: "res_skip", api });
    await delay(20, null);
    expect(skipLocalResearch("res_skip")).toBe(true);
    expect(await pending).toBeNull();
    expect(skipLocalResearch("res_skip")).toBe(false);
  });

  it("cancelling the ask cancels its deep job (AI-010)", async () => {
    const { api, cancelled } = slowApi(async (url) => page(url));
    const { bus } = recordingBus();
    const ask = new AbortController();
    const pending = runResearch("deep_agent", "q", { jobId: "res_ask", api, bus, signal: ask.signal });
    ask.abort();
    expect(await pending).toBeNull();
    expect(cancelled).toEqual(["res_ask"]);
  });

  it("also honours a polled isCancelled predicate", async () => {
    const { api, cancelled } = slowApi(async (url) => page(url));
    const { bus } = recordingBus();
    let cancelledAsk = false;
    const pending = runResearch("deep_agent", "q", {
      jobId: "res_poll",
      api,
      bus,
      isCancelled: () => cancelledAsk,
    });
    cancelledAsk = true;
    expect(await pending).toBeNull();
    expect(cancelled).toEqual(["res_poll"]);
  });

  it("requests only keyed tools and gives the agent a deadline inside the ask timeout", async () => {
    const { api, started } = slowApi(async (url) => page(url));
    const { bus } = recordingBus();
    const availability = { search: true, scrape: false, deepAgent: true };
    void runResearch("deep_agent", "q", { jobId: "res_tools", api, bus, availability, timeoutMs: 90_000 });
    await delay(0, null);
    expect(started[0]?.tools).toEqual(["exa_search"]);
    expect(started[0]?.deadlineMs).toBe(75_000);
    bus.emit("research.event", {
      type: "failed",
      jobId: "res_tools",
      error: { kind: "cancelled", code: "cancelled", message: "", recoverable: false },
    });
  });
});
