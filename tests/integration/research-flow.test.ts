/**
 * Research wiring in the ask pipeline: router decision → availability check →
 * privacy-scrubbed public query → search+scrape → citations on the response
 * and a context block in the prompt.
 */

import { createResponseEngine } from "@/ai/engine";
import { setTransport } from "@/lib/tauri/transport";
import type { ContextSnapshot, SearchResult } from "@/lib/types";
import { FakeTransport } from "../fixtures/helpers/fake-transport";
import { makeMode, makeSettings } from "../fixtures/helpers/builders";

const snapshot: ContextSnapshot = { timestamp: "2026-09-07T09:00:00.000Z" };

const results: SearchResult[] = [
  { id: "s1", title: "Vercel Pricing 2026", url: "https://vercel.com/pricing", snippet: "Pro is $20/seat", source: "mock" },
  { id: "s2", title: "Vercel pricing analysis", url: "https://blog.example/vercel", snippet: "A breakdown", source: "mock" },
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
});
