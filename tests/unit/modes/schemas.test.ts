import {
  outputSchemaFor,
  parseJsonLoose,
  parseStructuredOutput,
  repairTrailingCommas,
  stripWrappingFence,
} from "@/modes/schemas";
import type { ResponseSchemaId } from "@/lib/types";

const ALL_SCHEMAS: ResponseSchemaId[] = [
  "answer",
  "suggested-response",
  "behavioral",
  "coding",
  "system-design",
  "case",
  "sales",
  "recruiting",
  "meeting",
  "lecture",
];

describe("outputSchemaFor", () => {
  it("produces a named strict JSON schema for every response schema id", () => {
    for (const schemaId of ALL_SCHEMAS) {
      const spec = outputSchemaFor(schemaId);
      expect(spec.name).toBe(`bluey_${schemaId.replace(/-/g, "_")}`);
      expect(spec.strict).toBe(true);
      expect(spec.schema).toMatchObject({ type: "object" });
      const properties = spec.schema.properties as Record<string, unknown>;
      expect(properties.responseType).toBeDefined();
      expect(properties.content).toBeDefined();
    }
  });

  it("includes code for coding and diagram for system-design", () => {
    const coding = outputSchemaFor("coding").schema.properties as Record<string, unknown>;
    expect(coding.code).toBeDefined();
    const design = outputSchemaFor("system-design").schema.properties as Record<string, unknown>;
    expect(design.diagram).toBeDefined();
    const answer = outputSchemaFor("answer").schema.properties as Record<string, unknown>;
    expect(answer.code).toBeUndefined();
  });
});

describe("parseStructuredOutput tolerance", () => {
  it("parses clean structured JSON", () => {
    const parsed = parseStructuredOutput(
      "coding",
      JSON.stringify({
        responseType: "code",
        title: "Two Sum",
        content: "Use a hash map.",
        sections: [{ title: "Approach", content: "One pass." }],
        code: { language: "python", code: "def f(): pass" },
      }),
    );
    expect(parsed?.responseType).toBe("code");
    expect(parsed?.sections).toHaveLength(1);
    expect(parsed?.code?.language).toBe("python");
  });

  it("strips a wrapping markdown fence", () => {
    const parsed = parseStructuredOutput(
      "answer",
      '```json\n{"responseType":"answer","content":"Hello"}\n```',
    );
    expect(parsed?.content).toBe("Hello");
  });

  it("repairs trailing commas", () => {
    const parsed = parseStructuredOutput(
      "answer",
      '{"responseType":"answer","content":"Fixed",}',
    );
    expect(parsed?.content).toBe("Fixed");
  });

  it("extracts an embedded JSON object surrounded by prose", () => {
    const parsed = parseStructuredOutput(
      "answer",
      'Here you go:\n{"responseType":"answer","content":"Embedded"}\nHope that helps!',
    );
    expect(parsed?.content).toBe("Embedded");
  });

  it("falls back to a plain answer for non-JSON text", () => {
    const parsed = parseStructuredOutput("coding", "Just plain prose, no JSON at all.");
    expect(parsed).toEqual({ responseType: "answer", content: "Just plain prose, no JSON at all." });
  });

  it("returns null only for empty output", () => {
    expect(parseStructuredOutput("answer", "   \n ")).toBeNull();
  });

  it("substitutes the schema's response type when the model returns an invalid one", () => {
    const parsed = parseStructuredOutput(
      "sales",
      '{"responseType":"banana","content":"Say this."}',
    );
    expect(parsed?.responseType).toBe("suggestion");
  });

  it("builds content from sections when content is missing", () => {
    const parsed = parseStructuredOutput(
      "meeting",
      '{"responseType":"answer","sections":[{"title":"Decisions","content":"Ship Friday."}]}',
    );
    expect(parsed?.content).toContain("Decisions");
    expect(parsed?.content).toContain("Ship Friday.");
  });

  it("drops malformed sections and citations instead of failing", () => {
    const parsed = parseStructuredOutput(
      "answer",
      JSON.stringify({
        responseType: "answer",
        content: "ok",
        sections: [{ title: "Good", content: "kept" }, { bogus: true }, 42],
        citations: [{ title: "T", url: "https://x.test" }, { nope: 1 }],
      }),
    );
    expect(parsed?.sections).toHaveLength(1);
    expect(parsed?.citations).toHaveLength(1);
  });

  it("clamps confidence into 0..1", () => {
    const parsed = parseStructuredOutput("answer", '{"responseType":"answer","content":"x","confidence":3}');
    expect(parsed?.confidence).toBe(1);
  });

  it("reads null as absent — strict-mode providers answer every optional field", () => {
    // ChatGPT / Codex, Foundry and OpenAI-compatible endpoints are sent the optionals as
    // required nullable fields (bluey_protocols::json_schema) and return `null` for them.
    const parsed = parseStructuredOutput(
      "coding",
      JSON.stringify({
        responseType: "code",
        title: null,
        content: "Use a hash map.",
        sections: [{ title: "Approach", content: "One pass.", kind: null, language: null }],
        code: { language: "python", code: "def f(): pass", filename: null },
        diagram: null,
        confidence: null,
        citations: [{ title: "Docs", url: "https://x.test", snippet: null }],
      }),
    );
    expect(parsed?.responseType).toBe("code");
    expect(parsed?.title).toBeUndefined();
    expect(parsed?.content).toBe("Use a hash map.");
    expect(parsed?.sections).toEqual([{ title: "Approach", content: "One pass." }]);
    expect(parsed?.code).toEqual({ language: "python", code: "def f(): pass" });
    expect(parsed?.diagram).toBeUndefined();
    expect(parsed?.confidence).toBeUndefined();
    expect(parsed?.citations).toEqual([{ title: "Docs", url: "https://x.test" }]);

    const nulls = parseStructuredOutput("answer", '{"responseType":"answer","content":null,"sections":null,"code":null}');
    expect(nulls).toEqual({ responseType: "answer", content: '{"responseType":"answer","content":null,"sections":null,"code":null}' });
  });
});

describe("parse helpers", () => {
  it("stripWrappingFence removes fences with or without a language tag", () => {
    expect(stripWrappingFence('```json\n{"a":1}\n```')).toBe('{"a":1}');
    expect(stripWrappingFence('```\n{"a":1}\n```')).toBe('{"a":1}');
    expect(stripWrappingFence('{"a":1}')).toBe('{"a":1}');
  });

  it("repairTrailingCommas fixes objects and arrays", () => {
    expect(repairTrailingCommas('{"a":[1,2,],}')).toBe('{"a":[1,2]}');
  });

  it("parseJsonLoose combines all repairs", () => {
    expect(parseJsonLoose('```json\n{"a":1,}\n```')).toEqual({ a: 1 });
    expect(parseJsonLoose("not json")).toBeNull();
  });
});
