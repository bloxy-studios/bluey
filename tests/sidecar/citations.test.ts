import { describe, expect, it } from "vitest";

import { CitationStore, evidenceReport, normalizeUrl } from "../../sidecars/agent/src/citations";

describe("normalizeUrl", () => {
  it("lowercases the host, strips fragments and trailing slashes", () => {
    expect(normalizeUrl("https://Example.COM/Path/")).toBe("https://example.com/Path");
    expect(normalizeUrl("https://example.com/a#section")).toBe("https://example.com/a");
    expect(normalizeUrl("https://example.com/")).toBe("https://example.com");
  });

  it("keeps query strings (they can be significant)", () => {
    expect(normalizeUrl("https://example.com/a?p=1")).toBe("https://example.com/a?p=1");
  });

  it("falls back to a trimmed key for invalid URLs", () => {
    expect(normalizeUrl("  not a url// ")).toBe("not a url");
  });
});

describe("CitationStore", () => {
  it("dedupes by normalised URL and back-fills missing snippets", () => {
    const store = new CitationStore();
    store.add({ title: "A", url: "https://example.com/a/" });
    store.add({ title: "A duplicate", url: "https://EXAMPLE.com/a#frag", snippet: "later snippet" });
    store.add({ title: "B", url: "https://example.com/b", snippet: "b snippet" });

    const list = store.list();
    expect(list).toHaveLength(2);
    expect(list[0]).toEqual({ title: "A", url: "https://example.com/a/", snippet: "later snippet" });
    expect(list[1]?.title).toBe("B");
  });

  it("finalize drops model citations whose URL was never returned by a tool", () => {
    const store = new CitationStore();
    store.add({ title: "Seen", url: "https://example.com/seen", snippet: "from exa" });

    const citations = store.finalize([
      { title: "Model picked", url: "https://example.com/seen", snippet: "model snippet" },
      { title: "Invented", url: "https://evil.example.com/made-up" },
    ]);

    expect(citations.map((c) => c.url)).toEqual(["https://example.com/seen"]);
    expect(citations[0]?.title).toBe("Model picked");
    expect(citations[0]?.snippet).toBe("model snippet");
  });

  it("finalize returns only the cited sources, not every search hit", () => {
    const store = new CitationStore();
    for (let i = 0; i < 20; i += 1) store.add({ title: `Hit ${i}`, url: `https://example.com/${i}` });

    const citations = store.finalize([
      { title: "One (model)", url: "https://example.com/1/" },
      { title: "Two", url: "https://example.com/2" },
    ]);
    expect(citations.map((c) => c.url)).toEqual(["https://example.com/1", "https://example.com/2"]);
    expect(citations[0]?.title).toBe("One (model)");
  });

  it("finalize adds known sources linked from the report body", () => {
    const store = new CitationStore();
    store.add({ title: "One", url: "https://example.com/1" });
    store.add({ title: "Two", url: "https://example.com/2" });
    const report = "See [two](https://example.com/2) and https://invented.example/x.";
    expect(store.finalize([], report).map((c) => c.url)).toEqual(["https://example.com/2"]);
  });

  it("finalize with nothing cited falls back to the pages read in full", () => {
    const store = new CitationStore();
    store.add({ title: "Snippet only", url: "https://example.com/1" });
    store.add({ title: "Scraped", url: "https://example.com/2" }, { fetched: true });
    expect(store.finalize(undefined).map((c) => c.title)).toEqual(["Scraped"]);
  });

  it("sanitizeReport de-links URLs the tools never returned and keeps known ones", () => {
    const store = new CitationStore();
    store.add({ title: "Seen", url: "https://example.com/seen" });
    const report =
      "Per [the study](https://example.com/seen/), growth doubled [src](https://made.up/a). " +
      "More at https://www.invented.example/page, and https://example.com/seen.";
    expect(store.sanitizeReport(report)).toBe(
      "Per [the study](https://example.com/seen/), growth doubled src. " +
        "More at invented.example, and https://example.com/seen.",
    );
  });

  it("sanitizeReport keeps known URLs with balanced parentheses intact", () => {
    const store = new CitationStore();
    store.add({ title: "Mercury", url: "https://en.wikipedia.org/wiki/Mercury_(planet)" });
    const report =
      "[Wikipedia](https://en.wikipedia.org/wiki/Mercury_(planet)). " +
      "See https://en.wikipedia.org/wiki/Mercury_(planet) too (or https://made.up/a).";
    expect(store.sanitizeReport(report)).toBe(
      "[Wikipedia](https://en.wikipedia.org/wiki/Mercury_(planet)). " +
        "See https://en.wikipedia.org/wiki/Mercury_(planet) too (or made.up).",
    );
    expect(store.finalize([], report).map((c) => c.url)).toEqual([
      "https://en.wikipedia.org/wiki/Mercury_(planet)",
    ]);
  });

  it("sanitizeReport leaves URLs inside code spans and fenced blocks alone", () => {
    const store = new CitationStore();
    const report =
      "Run `curl https://api.example.com/v1` first.\n\n```sh\nwget https://cdn.example.com/x.tgz\n```\n" +
      "Then https://made.up/a.";
    expect(store.sanitizeReport(report)).toBe(
      "Run `curl https://api.example.com/v1` first.\n\n```sh\nwget https://cdn.example.com/x.tgz\n```\n" +
        "Then made.up.",
    );
  });

  it("clamps very long snippets", () => {
    const store = new CitationStore();
    store.add({ title: "Long", url: "https://example.com/long", snippet: "x".repeat(2000) });
    expect(store.list()[0]?.snippet?.length).toBeLessThanOrEqual(400);
  });
});

describe("evidenceReport", () => {
  it("lists the gathered sources as links the citation check accepts", () => {
    const store = new CitationStore();
    store.add({ title: "A [draft]", url: "https://example.com/a", snippet: "first" });
    store.add({ title: "B", url: "https://example.com/b" });
    const report = evidenceReport("turns", store.list());
    expect(report).toContain("ran out of turns");
    expect(report).toContain("- [A draft](https://example.com/a) — first");
    expect(store.sanitizeReport(report)).toBe(report);
    expect(store.finalize(undefined, report)).toHaveLength(2);
  });

  it("survives the sanitizer when a source URL has parentheses", () => {
    const store = new CitationStore();
    store.add({ title: "Mercury (planet)", url: "https://en.wikipedia.org/wiki/Mercury_(planet)" });
    store.add({ title: "B", url: "https://example.com/b" });
    const report = evidenceReport("time", store.list());
    expect(report).toContain("- [Mercury (planet)](https://en.wikipedia.org/wiki/Mercury_(planet))");
    expect(store.sanitizeReport(report)).toBe(report);
    expect(store.finalize(undefined, report)).toHaveLength(2);
  });
});
