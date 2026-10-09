import { describe, expect, it } from "vitest";

import type { DetectedEvent, DetectedEventType } from "@/lib/types";
import {
  QUESTION_STALE_MS,
  SURFACE_COOLDOWN_MS,
  SURFACE_DEDUPE_WINDOW_MS,
  shouldSurface,
  type SurfaceContext,
} from "@/stores/proactive";
import { makeMode } from "../fixtures/helpers/builders";

const NOW = Date.parse("2026-09-28T10:00:00.000Z");
const interview = makeMode({ id: "interview", responseSchema: "suggested-response" });
const general = makeMode({ id: "general", responseSchema: "answer" });

function event(text: string, overrides: Partial<DetectedEvent> = {}): DetectedEvent {
  return {
    id: `evt-${text}`,
    type: "question" as DetectedEventType,
    confidence: 0.82,
    requiresResponse: true,
    text,
    segmentIds: [],
    speaker: "Interviewer",
    detectedAt: new Date(NOW).toISOString(),
    ...overrides,
  };
}

function ctx(overrides: Partial<SurfaceContext> = {}): SurfaceContext {
  return { mode: interview, now: NOW, surfaced: [], dismissals: 0, ...overrides };
}

describe("shouldSurface (LIVE-009)", () => {
  it.each([
    "Right?",
    "Okay?",
    "So, what?",
    "Can you hear me?",
    "Is my screen visible?",
    "Okay, can you see my screen?",
    "Does that make sense?",
    "Any questions so far?",
    "Could you repeat that?",
    "You know?",
  ])("drops back-channel and call-meta %j", (text) => {
    expect(shouldSurface(event(text), ctx())).toBe("not_substantive");
  });

  it.each([
    "Why React?",
    "What's your biggest weakness?",
    "How would you design a rate limiter?",
    "So, how did you measure the latency win?",
  ])("surfaces a real interview question %j", (text) => {
    expect(shouldSurface(event(text), ctx())).toBe("surface");
  });

  it("keeps a short specific detection (an objection carries meaning on its own)", () => {
    const objection = event("Too expensive.", { type: "objection", confidence: 0.86 });
    expect(shouldSurface(objection, ctx({ mode: makeMode({ id: "sales", responseSchema: "sales" }) }))).toBe(
      "surface",
    );
  });

  it("answers only direct questions outside conversational modes", () => {
    expect(shouldSurface(event("What is the rollout date?"), ctx({ mode: general }))).toBe("surface");
    const implied = event("how we handle the rollout is open", { confidence: 0.58 });
    expect(shouldSurface(implied, ctx({ mode: general }))).toBe("below_threshold");
    const decision = event("We decided to ship on Friday", { type: "decision", confidence: 0.86 });
    expect(shouldSurface(decision, ctx({ mode: general }))).toBe("below_threshold");
  });

  it("surfaces a near-identical question once per window", () => {
    const earlier = { text: "How would you design a rate limiter?", speaker: "Interviewer", at: NOW - 30_000 };
    const repeat = event("So how would you design a rate limiter?");
    expect(shouldSurface(repeat, ctx({ surfaced: [earlier] }))).toBe("duplicate");
    const expired = { ...earlier, at: NOW - SURFACE_DEDUPE_WINDOW_MS - 1 };
    expect(shouldSurface(repeat, ctx({ surfaced: [expired] }))).toBe("surface");
  });

  it("cools down after a suggestion unless the speaker changes or the type is specific", () => {
    const last = { text: "Tell me about your last project?", speaker: "Interviewer", at: NOW - 3_000 };
    const next = event("Which part did you own?");
    expect(shouldSurface(next, ctx({ surfaced: [last] }))).toBe("cooldown");
    expect(shouldSurface({ ...next, speaker: "Panelist" }, ctx({ surfaced: [last] }))).toBe("surface");
    const coding = { ...next, type: "coding_problem" as const, confidence: 0.9 };
    expect(shouldSurface(coding, ctx({ surfaced: [last] }))).toBe("surface");
    const later = { ...last, at: NOW - SURFACE_COOLDOWN_MS };
    expect(shouldSurface(next, ctx({ surfaced: [later] }))).toBe("surface");
  });

  it("lets the same speaker correct a question inside the cooldown", () => {
    const last = { text: "How do you size a thread pool?", speaker: "Interviewer", at: NOW - 2_000 };
    const correction = event("Sorry, I mean how do you size a connection pool?");
    expect(shouldSurface(correction, ctx({ surfaced: [last] }))).toBe("surface");
  });

  it("drops a question that waited past the staleness window", () => {
    const old = event("Which database would you pick?", {
      detectedAt: new Date(NOW - QUESTION_STALE_MS - 1).toISOString(),
    });
    expect(shouldSurface(old, ctx())).toBe("stale");
  });

  it("raises the bar after dismissals without muting confident questions", () => {
    const implied = event("walk me through the migration plan", { confidence: 0.6 });
    expect(shouldSurface(implied, ctx())).toBe("surface");
    expect(shouldSurface(implied, ctx({ dismissals: 3 }))).toBe("below_threshold");
    expect(shouldSurface(event("Why did you pick Postgres?"), ctx({ dismissals: 10 }))).toBe("surface");
  });
});
