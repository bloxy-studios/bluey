import { describe, expect, it } from "vitest";

import { ToolError } from "../../sidecars/agent/src/tools/errors";
import {
  createFirecrawlClient,
  mapFirecrawlResponse,
} from "../../sidecars/agent/src/tools/firecrawl";

const sampleResponse = {
  success: true,
  data: {
    markdown: "# Heading\n\nBody text.",
    metadata: { title: "Page title", sourceURL: "https://example.com/final" },
  },
};

describe("mapFirecrawlResponse", () => {
  it("maps markdown, title and the resolved sourceURL", () => {
    const result = mapFirecrawlResponse(sampleResponse, "https://example.com/requested");
    expect(result).toEqual({
      url: "https://example.com/final",
      title: "Page title",
      markdown: "# Heading\n\nBody text.",
      truncated: false,
    });
  });

  it("falls back to the requested URL when metadata is missing", () => {
    const result = mapFirecrawlResponse(
      { success: true, data: { markdown: "text" } },
      "https://example.com/requested",
    );
    expect(result.url).toBe("https://example.com/requested");
    expect(result.title).toBeUndefined();
  });

  it("truncates oversized markdown and flags it", () => {
    const big = { success: true, data: { markdown: "y".repeat(50) } };
    const result = mapFirecrawlResponse(big, "https://e.com", 10);
    expect(result.truncated).toBe(true);
    expect(result.markdown.startsWith("yyyyyyyyyy\n\n[…")).toBe(true);
  });

  it("throws typed errors for failure bodies and empty content", () => {
    expect(() =>
      mapFirecrawlResponse({ success: false, error: "denied" }, "https://e.com"),
    ).toThrowError(/denied/);
    expect(() =>
      mapFirecrawlResponse({ success: true, data: { markdown: "  " } }, "https://e.com"),
    ).toThrowError(ToolError);
  });
});

describe("createFirecrawlClient", () => {
  it("sends the documented v2 scrape request with a Bearer token", async () => {
    let captured: { url: string; init?: RequestInit } | undefined;
    const fetchImpl = async (url: string | URL, init?: RequestInit) => {
      captured = { url: String(url), init };
      return new Response(JSON.stringify(sampleResponse), { status: 200 });
    };

    const client = createFirecrawlClient({ apiKey: "fc-key", fetchImpl });
    const page = await client.scrape("https://example.com/requested");

    expect(page.markdown).toContain("Body text");
    expect(captured?.url).toBe("https://api.firecrawl.dev/v2/scrape");
    const headers = captured?.init?.headers as Record<string, string>;
    expect(headers["authorization"]).toBe("Bearer fc-key");
    expect(JSON.parse(String(captured?.init?.body))).toEqual({
      url: "https://example.com/requested",
      formats: ["markdown"],
      onlyMainContent: true,
    });
  });

  it("maps HTTP errors to typed ToolErrors", async () => {
    const fetchImpl = async () => new Response("nope", { status: 500 });
    const client = createFirecrawlClient({ apiKey: "k", fetchImpl });
    const err = await client.scrape("https://example.com").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ToolError);
    expect((err as ToolError).code).toBe("http_error");
  });

  it("refuses to construct without an API key", () => {
    expect(() => createFirecrawlClient({ apiKey: "" })).toThrowError(/FIRECRAWL_API_KEY/);
  });
});
