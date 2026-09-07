import { describe, expect, it } from "vitest";

import { ToolError, toToolError } from "../../sidecars/agent/src/tools/errors";
import { createExaClient, mapExaResponse } from "../../sidecars/agent/src/tools/exa";

const sampleResponse = {
  results: [
    {
      id: "r1",
      url: "https://example.com/one",
      title: "Result one",
      publishedDate: "2026-01-02",
      author: "Ada",
      summary: "A summary of one.",
      highlights: ["ignored when summary present"],
    },
    {
      id: "r2",
      url: "https://example.com/two",
      title: "",
      highlights: ["first highlight", "second highlight"],
    },
    { id: "broken-no-url", title: "no url, skipped" },
  ],
};

describe("mapExaResponse", () => {
  it("maps results, preferring summary over highlights", () => {
    const items = mapExaResponse(sampleResponse);
    expect(items).toHaveLength(2);
    expect(items[0]).toMatchObject({
      title: "Result one",
      url: "https://example.com/one",
      snippet: "A summary of one.",
      publishedDate: "2026-01-02",
      author: "Ada",
    });
    // empty title falls back to the URL; highlights are joined
    expect(items[1]).toMatchObject({
      title: "https://example.com/two",
      snippet: "first highlight … second highlight",
    });
  });

  it("throws a typed error on malformed bodies", () => {
    expect(() => mapExaResponse(null)).toThrowError(ToolError);
    expect(() => mapExaResponse({ nope: true })).toThrowError(/no results array/);
  });
});

describe("createExaClient", () => {
  it("sends the documented request shape with the x-api-key header", async () => {
    let captured: { url: string; init?: RequestInit } | undefined;
    const fetchImpl = async (url: string | URL, init?: RequestInit) => {
      captured = { url: String(url), init };
      return new Response(JSON.stringify(sampleResponse), { status: 200 });
    };

    const client = createExaClient({ apiKey: "exa-key", fetchImpl });
    const items = await client.search({
      query: "bluey research",
      numResults: 5,
      startPublishedDate: "2026-01-01",
    });

    expect(items).toHaveLength(2);
    expect(captured?.url).toBe("https://api.exa.ai/search");
    const headers = captured?.init?.headers as Record<string, string>;
    expect(headers["x-api-key"]).toBe("exa-key");
    expect(headers["content-type"]).toBe("application/json");
    const body = JSON.parse(String(captured?.init?.body));
    expect(body).toEqual({
      query: "bluey research",
      type: "auto",
      numResults: 5,
      contents: { highlights: { maxCharacters: 600 }, summary: true },
      startPublishedDate: "2026-01-01",
    });
  });

  it("clamps numResults into 1..10", async () => {
    let body: Record<string, unknown> = {};
    const fetchImpl = async (_url: string | URL, init?: RequestInit) => {
      body = JSON.parse(String(init?.body));
      return new Response(JSON.stringify({ results: [] }), { status: 200 });
    };
    const client = createExaClient({ apiKey: "k", fetchImpl });
    await client.search({ query: "q", numResults: 99 });
    expect(body["numResults"]).toBe(10);
    await client.search({ query: "q", numResults: 0 });
    expect(body["numResults"]).toBe(1);
  });

  it("maps HTTP failures to a typed http_error without leaking the key", async () => {
    const fetchImpl = async () => new Response("rate limited", { status: 429 });
    const client = createExaClient({ apiKey: "super-secret", fetchImpl });
    const err = await client.search({ query: "q" }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ToolError);
    expect((err as ToolError).code).toBe("http_error");
    expect((err as ToolError).status).toBe(429);
    expect((err as ToolError).message).not.toContain("super-secret");
  });

  it("maps network failures to network_error", async () => {
    const fetchImpl = async () => {
      throw new Error("socket hang up");
    };
    const client = createExaClient({ apiKey: "k", fetchImpl });
    const err = await client.search({ query: "q" }).catch((e: unknown) => e);
    expect((err as ToolError).code).toBe("network_error");
  });

  it("refuses to construct without an API key", () => {
    expect(() => createExaClient({ apiKey: "" })).toThrowError(/EXA_API_KEY/);
  });
});

describe("toToolError timeout mapping", () => {
  it("maps AbortError/TimeoutError to a timeout ToolError", () => {
    const abort = new Error("aborted");
    abort.name = "AbortError";
    expect(toToolError(abort, "exa timed out").code).toBe("timeout");
    const timeout = new Error("timed out");
    timeout.name = "TimeoutError";
    expect(toToolError(timeout, "exa timed out").code).toBe("timeout");
  });
});
