import {
  defaultTaskFor,
  effectiveStyle,
  isCandidateMode,
  requires,
  validateModeDraft,
} from "@/modes/registry";
import { makeMode, makeSettings } from "../../fixtures/helpers/builders";

describe("isCandidateMode", () => {
  it("treats interview-side built-ins as candidate modes", () => {
    for (const id of ["interview", "behavioral-interview", "coding-interview", "system-design", "case-interview"]) {
      expect(isCandidateMode(makeMode({ id }))).toBe(true);
    }
  });

  it("never treats recruiting as a candidate mode", () => {
    expect(isCandidateMode(makeMode({ id: "recruiting", responseSchema: "recruiting" }))).toBe(false);
  });

  it("uses group and schema hints for custom modes", () => {
    expect(isCandidateMode(makeMode({ id: "custom-1", group: "Looking for work" }))).toBe(true);
    expect(isCandidateMode(makeMode({ id: "custom-2", responseSchema: "behavioral" }))).toBe(true);
    expect(isCandidateMode(makeMode({ id: "custom-3", responseSchema: "meeting" }))).toBe(false);
  });
});

describe("requires / effectiveStyle / defaultTaskFor", () => {
  it("checks context requirements", () => {
    const mode = makeMode({ contextRequirements: ["screen", "transcript"] });
    expect(requires(mode, "screen")).toBe(true);
    expect(requires(mode, "resume")).toBe(false);
  });

  it("merges mode style overrides over user settings", () => {
    const settings = makeSettings();
    expect(effectiveStyle(makeMode(), settings)).toEqual({ length: "balanced", tone: "natural" });
    expect(effectiveStyle(makeMode({ responseStyle: { tone: "technical" } }), settings)).toEqual({
      length: "balanced",
      tone: "technical",
    });
  });

  it("derives the default task from the response schema", () => {
    expect(defaultTaskFor(makeMode({ responseSchema: "coding" }))).toBe("coding");
    expect(defaultTaskFor(makeMode({ responseSchema: "system-design" }))).toBe("system_design");
    expect(defaultTaskFor(makeMode({ responseSchema: "sales" }))).toBe("answer");
  });
});

describe("validateModeDraft", () => {
  it("accepts a sane draft", () => {
    const result = validateModeDraft({
      name: "Standup Helper",
      icon: "users",
      responseSchema: "meeting",
      preferredLatency: "fast",
      contextRequirements: ["transcript"],
      responseStyle: { length: "concise", tone: "direct" },
    });
    expect(result).toEqual({ ok: true, errors: [] });
  });

  it("rejects empty and oversized names", () => {
    expect(validateModeDraft({ name: "  " }).ok).toBe(false);
    expect(validateModeDraft({ name: "x".repeat(61) }).ok).toBe(false);
  });

  it("rejects bad icons, schemas, latencies, requirements and styles", () => {
    const result = validateModeDraft({
      name: "Bad",
      icon: "Not An Icon",
      responseSchema: "poem" as never,
      preferredLatency: "warp" as never,
      contextRequirements: ["screen", "telepathy" as never],
      responseStyle: { length: "epic" as never },
    });
    expect(result.ok).toBe(false);
    expect(result.errors.length).toBeGreaterThanOrEqual(5);
  });
});
