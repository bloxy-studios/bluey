import { describe, expect, it } from "vitest";

import {
  deepResearchRequestSchema,
  parseRequestLine,
  ProtocolWriter,
  wireError,
} from "../../sidecars/agent/src/protocol";

function collector() {
  const lines: string[] = [];
  return {
    lines,
    sink: {
      write(chunk: string) {
        lines.push(chunk);
        return true;
      },
    },
  };
}

describe("parseRequestLine", () => {
  it("parses a valid request", () => {
    const parsed = parseRequestLine('{"id":7,"method":"research.run","params":{"a":1}}');
    expect(parsed).toEqual({
      ok: true,
      request: { id: 7, method: "research.run", params: { a: 1 } },
    });
  });

  it("accepts string ids", () => {
    const parsed = parseRequestLine('{"id":"req-1","method":"research.cancel"}');
    expect(parsed.ok).toBe(true);
    if (parsed.ok) expect(parsed.request.id).toBe("req-1");
  });

  it("rejects invalid JSON without throwing", () => {
    const parsed = parseRequestLine("{nope");
    expect(parsed).toEqual({ ok: false, error: "invalid JSON", id: null });
  });

  it("rejects non-object frames", () => {
    expect(parseRequestLine("[1,2]").ok).toBe(false);
    expect(parseRequestLine('"hi"').ok).toBe(false);
  });

  it("rejects a missing method but keeps the id for the error reply", () => {
    const parsed = parseRequestLine('{"id":3}');
    expect(parsed).toEqual({ ok: false, error: "missing method", id: 3 });
  });

  it("rejects a missing id", () => {
    const parsed = parseRequestLine('{"method":"research.run"}');
    expect(parsed).toEqual({ ok: false, error: "missing id", id: null });
  });
});

describe("ProtocolWriter", () => {
  it("writes results, errors and events as single JSON lines", () => {
    const { lines, sink } = collector();
    const writer = new ProtocolWriter(sink);

    writer.result(1, { accepted: true });
    writer.error("x", wireError("unknown_method", "nope", "sidecar"));
    writer.event("research.progress", { jobId: "j", message: "hello\nworld" });

    expect(lines).toHaveLength(3);
    for (const line of lines) {
      expect(line.endsWith("\n")).toBe(true);
      // exactly one line per frame — embedded newlines must be escaped
      expect(line.slice(0, -1)).not.toContain("\n");
      expect(() => JSON.parse(line)).not.toThrow();
    }
    expect(JSON.parse(lines[0]!)).toEqual({ id: 1, result: { accepted: true } });
    expect(JSON.parse(lines[1]!)).toEqual({
      id: "x",
      error: { code: "unknown_method", message: "nope", kind: "sidecar" },
    });
    expect(JSON.parse(lines[2]!)).toEqual({
      event: "research.progress",
      data: { jobId: "j", message: "hello\nworld" },
    });
  });
});

describe("deepResearchRequestSchema", () => {
  const valid = {
    jobId: "job-1",
    query: "public query",
    goal: "answer something",
    tools: ["exa_search"],
  };

  it("accepts a minimal valid request", () => {
    expect(deepResearchRequestSchema.safeParse(valid).success).toBe(true);
  });

  it("accepts the full shape from the protocol doc", () => {
    const parsed = deepResearchRequestSchema.safeParse({
      ...valid,
      sessionId: "s-1",
      maxTurns: 12,
      tools: ["exa_search", "firecrawl_scrape", "document_read"],
      allowedDocumentIds: ["doc-1"],
      model: "claude-sonnet-5",
    });
    expect(parsed.success).toBe(true);
  });

  it("rejects unknown tools and empty tool lists", () => {
    expect(deepResearchRequestSchema.safeParse({ ...valid, tools: ["Bash"] }).success).toBe(false);
    expect(deepResearchRequestSchema.safeParse({ ...valid, tools: [] }).success).toBe(false);
  });

  it("rejects a missing query/goal", () => {
    expect(deepResearchRequestSchema.safeParse({ ...valid, query: "" }).success).toBe(false);
    const { goal: _goal, ...noGoal } = { ...valid, goal: "g" };
    expect(deepResearchRequestSchema.safeParse(noGoal).success).toBe(false);
  });
});
