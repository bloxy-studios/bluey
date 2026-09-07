import { classifySegment, isQuestionText } from "@/transcript/classifier";
import { makeMode, makeSegment } from "../../fixtures/helpers/builders";
import { loadAllFixtures } from "../../fixtures/helpers/fixtures";

const NOW = () => new Date("2026-09-07T09:01:00.000Z");
const idGen = () => "fixed-id";

describe("classifySegment per fixture", () => {
  for (const fixture of loadAllFixtures()) {
    it(`detects ${fixture.expected.detectedEventType} in the ${fixture.name} fixture`, () => {
      const segments = fixture.transcript;
      const segment = segments[segments.length - 1]!;
      const event = classifySegment({
        segment,
        recent: segments.slice(0, -1),
        mode: fixture.mode,
        now: NOW,
        idGen,
      });
      expect(event).not.toBeNull();
      expect(event?.type).toBe(fixture.expected.detectedEventType);
      expect(event?.requiresResponse).toBe(fixture.expected.requiresResponse);
      expect(event?.text).toBe(segment.text.trim());
      expect(event?.segmentIds).toEqual([segment.id]);
      expect(event?.confidence).toBeGreaterThan(0.5);
      expect(event?.id).toBe("evt_fixed-id");
    });
  }
});

describe("classifySegment rules", () => {
  const interviewMode = makeMode({ id: "interview", responseSchema: "suggested-response", group: "Looking for work" });

  it("does not require a response for the user's own questions", () => {
    const event = classifySegment({
      segment: makeSegment({ source: "microphone", text: "Should I walk you through my approach?" }),
      recent: [],
      mode: interviewMode,
      now: NOW,
      idGen,
    });
    expect(event?.requiresResponse).toBe(false);
    expect(event?.speaker).toBe("You");
  });

  it("does not require responses in meeting mode even for others' questions", () => {
    const event = classifySegment({
      segment: makeSegment({ source: "system", text: "Who is taking notes today?" }),
      recent: [],
      mode: makeMode({ id: "team-meeting", responseSchema: "meeting" }),
      now: NOW,
      idGen,
    });
    expect(event?.type).toBe("question");
    expect(event?.requiresResponse).toBe(false);
  });

  it("ignores unfinalized partials and empty text", () => {
    expect(
      classifySegment({
        segment: makeSegment({ finalized: false, text: "Why would" }),
        recent: [],
        mode: interviewMode,
        now: NOW,
        idGen,
      }),
    ).toBeNull();
    expect(
      classifySegment({
        segment: makeSegment({ text: " " }),
        recent: [],
        mode: interviewMode,
        now: NOW,
        idGen,
      }),
    ).toBeNull();
  });

  it("returns null for plain statements", () => {
    expect(
      classifySegment({
        segment: makeSegment({ source: "system", text: "We moved the office to the fourth floor last month." }),
        recent: [],
        mode: interviewMode,
        now: NOW,
        idGen,
      }),
    ).toBeNull();
  });

  it("labels a rapid same-speaker second question as a follow_up", () => {
    const first = makeSegment({
      id: "q1",
      source: "system",
      text: "What databases have you used?",
      startTime: 0,
      endTime: 3000,
    });
    const second = makeSegment({
      id: "q2",
      source: "system",
      text: "And which one would you pick today?",
      startTime: 4000,
      endTime: 6500,
    });
    const event = classifySegment({ segment: second, recent: [first], mode: interviewMode, now: NOW, idGen });
    expect(event?.type).toBe("follow_up");
    expect(event?.requiresResponse).toBe(true);
  });

  it("detects buying signals separately from objections", () => {
    const salesMode = makeMode({ id: "sales", responseSchema: "sales" });
    const buying = classifySegment({
      segment: makeSegment({ source: "system", text: "How soon could we start a pilot?" }),
      recent: [],
      mode: salesMode,
      now: NOW,
      idGen,
    });
    expect(buying?.type).toBe("buying_signal");
    expect(buying?.requiresResponse).toBe(true);
  });

  it("detects competitor mentions when a competitor list is provided", () => {
    const salesMode = makeMode({ id: "sales", responseSchema: "sales" });
    const event = classifySegment({
      segment: makeSegment({ source: "system", text: "We have been evaluating FlowMetrics as well." }),
      recent: [],
      mode: salesMode,
      competitorNames: ["FlowMetrics"],
      now: NOW,
      idGen,
    });
    expect(event?.type).toBe("competitor_mention");
  });

  it("detects action items with owners and deadlines", () => {
    const event = classifySegment({
      segment: makeSegment({ source: "microphone", text: "I'll send the migration checklist by Friday." }),
      recent: [],
      mode: makeMode({ id: "team-meeting", responseSchema: "meeting" }),
      now: NOW,
      idGen,
    });
    expect(event?.type).toBe("action_item");
    expect(event?.requiresResponse).toBe(false);
  });
});

describe("isQuestionText", () => {
  it("scores explicit question marks highest", () => {
    const withMark = isQuestionText("Are you ready?");
    const rising = isQuestionText("walk me through your resume");
    const lead = isQuestionText("what happens after the beta ends");
    expect(withMark.confidence).toBeGreaterThan(rising.confidence);
    expect(rising.confidence).toBeGreaterThan(lead.confidence);
    expect(isQuestionText("This is a statement.").question).toBe(false);
  });
});
