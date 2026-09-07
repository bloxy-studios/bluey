import { dedupePartials, recentSegments, summarizeOlder } from "@/transcript/window";
import { makeSegment } from "../../fixtures/helpers/builders";

describe("recentSegments", () => {
  const segments = [
    makeSegment({ id: "a", startTime: 0, endTime: 5_000 }),
    makeSegment({ id: "b", startTime: 60_000, endTime: 65_000 }),
    makeSegment({ id: "c", startTime: 170_000, endTime: 175_000 }),
  ];

  it("keeps only segments inside the window relative to the newest segment", () => {
    expect(recentSegments(segments, 120).map((s) => s.id)).toEqual(["b", "c"]);
    expect(recentSegments(segments, 30).map((s) => s.id)).toEqual(["c"]);
    expect(recentSegments(segments, 1000).map((s) => s.id)).toEqual(["a", "b", "c"]);
  });

  it("accepts an explicit now reference", () => {
    expect(recentSegments(segments, 30, 66_000).map((s) => s.id)).toEqual(["b"]);
  });

  it("handles empty input", () => {
    expect(recentSegments([], 60)).toEqual([]);
  });
});

describe("dedupePartials", () => {
  it("drops a partial superseded by a later overlapping segment from the same source", () => {
    const partial = makeSegment({ id: "p1", finalized: false, text: "Why do", startTime: 0, endTime: 1_000 });
    const final = makeSegment({ id: "f1", finalized: true, text: "Why do you want this job?", startTime: 0, endTime: 2_500 });
    expect(dedupePartials([partial, final]).map((s) => s.id)).toEqual(["f1"]);
  });

  it("drops a partial when a finalized segment shares its id", () => {
    const partial = makeSegment({ id: "same", finalized: false, text: "Hel", startTime: 0, endTime: 900 });
    const final = makeSegment({ id: "same", finalized: true, text: "Hello there", startTime: 5_000, endTime: 6_000 });
    const result = dedupePartials([partial, final]);
    expect(result).toHaveLength(1);
    expect(result[0]?.finalized).toBe(true);
  });

  it("keeps the latest live partial per source", () => {
    const done = makeSegment({ id: "f", finalized: true, startTime: 0, endTime: 1_000 });
    const live = makeSegment({ id: "p", finalized: false, text: "still speak", startTime: 2_000, endTime: 3_000 });
    expect(dedupePartials([done, live]).map((s) => s.id)).toEqual(["f", "p"]);
  });

  it("does not let a microphone segment supersede a system partial", () => {
    const systemPartial = makeSegment({ id: "sp", source: "system", finalized: false, startTime: 0, endTime: 2_000 });
    const micFinal = makeSegment({ id: "mf", source: "microphone", finalized: true, startTime: 500, endTime: 1_500 });
    expect(dedupePartials([systemPartial, micFinal]).map((s) => s.id)).toEqual(["sp", "mf"]);
  });
});

describe("summarizeOlder", () => {
  it("keeps one first-sentence line per speaker turn", () => {
    const summary = summarizeOlder([
      makeSegment({ speaker: "Interviewer", source: "system", text: "Tell me about the outage. What happened next?" }),
      makeSegment({ speaker: "Interviewer", source: "system", text: "Same turn continues." }),
      makeSegment({ speaker: "You", source: "microphone", text: "We lost the primary database. Then we failed over." }),
    ]);
    const lines = summary.split("\n");
    expect(lines).toHaveLength(2);
    expect(lines[0]).toBe("Interviewer: Tell me about the outage.");
    expect(lines[1]).toBe("You: We lost the primary database.");
  });

  it("returns an empty string for no segments", () => {
    expect(summarizeOlder([])).toBe("");
  });
});
