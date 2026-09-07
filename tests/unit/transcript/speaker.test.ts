import { counterpartLabelFor, labelSpeaker } from "@/transcript/speaker";
import { makeMode, makeSegment } from "../../fixtures/helpers/builders";

describe("labelSpeaker", () => {
  it("labels microphone audio as You with high (but not certain) confidence", () => {
    const label = labelSpeaker(makeSegment({ source: "microphone" }), makeMode());
    expect(label).toEqual({ speaker: "You", confidence: 0.95 });
  });

  it("labels system audio by mode with moderate confidence", () => {
    expect(labelSpeaker(makeSegment({ source: "system" }), makeMode({ id: "interview" }))).toEqual({
      speaker: "Interviewer",
      confidence: 0.6,
    });
    expect(
      labelSpeaker(makeSegment({ source: "system" }), makeMode({ id: "sales", responseSchema: "sales" })),
    ).toEqual({ speaker: "Customer", confidence: 0.6 });
    expect(
      labelSpeaker(makeSegment({ source: "system" }), makeMode({ id: "recruiting", responseSchema: "recruiting" })),
    ).toEqual({ speaker: "Candidate", confidence: 0.6 });
    expect(labelSpeaker(makeSegment({ source: "system" }), makeMode())).toEqual({
      speaker: "Speaker",
      confidence: 0.6,
    });
  });

  it("keeps an existing label when its confidence beats the heuristic", () => {
    const label = labelSpeaker(
      makeSegment({ source: "system", speaker: "Speaker 2", speakerConfidence: 0.8 }),
      makeMode(),
    );
    expect(label.speaker).toBe("Speaker 2");
    expect(label.confidence).toBe(0.8);
  });

  it("never returns certainty 1.0", () => {
    const label = labelSpeaker(
      makeSegment({ source: "system", speaker: "Diarized", speakerConfidence: 1 }),
      makeMode(),
    );
    expect(label.confidence).toBeLessThan(1);
  });

  it("maps lecture mode to Lecturer", () => {
    expect(counterpartLabelFor(makeMode({ id: "lecture", responseSchema: "lecture" }))).toBe("Lecturer");
  });
});
