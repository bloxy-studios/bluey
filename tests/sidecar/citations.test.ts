import { describe, expect, it } from "vitest";

import { CitationStore, normalizeUrl } from "../../sidecars/agent/src/citations";

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

  it("finalize appends observed sources the model did not cite (deduped)", () => {
    const store = new CitationStore();
    store.add({ title: "One", url: "https://example.com/1" });
    store.add({ title: "Two", url: "https://example.com/2" });

    const citations = store.finalize([{ title: "One (model)", url: "https://example.com/1/" }]);
    expect(citations.map((c) => c.url)).toEqual(["https://example.com/1", "https://example.com/2"]);
    expect(citations[0]?.title).toBe("One (model)");
  });

  it("finalize without model citations returns every observed source", () => {
    const store = new CitationStore();
    store.add({ title: "One", url: "https://example.com/1" });
    store.add({ title: "Two", url: "https://example.com/2" });
    expect(store.finalize(undefined)).toHaveLength(2);
  });

  it("clamps very long snippets", () => {
    const store = new CitationStore();
    store.add({ title: "Long", url: "https://example.com/long", snippet: "x".repeat(2000) });
    expect(store.list()[0]?.snippet?.length).toBeLessThanOrEqual(400);
  });
});
