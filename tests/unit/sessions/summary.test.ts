import { generateSessionSummary, parseSummaryOutput, renderSummaryInput, summaryOutputSchema } from "@/sessions/summary";
import { buildTimeline } from "@/sessions/timeline";
import { composeSessionMarkdown } from "@/sessions/export";
import type { SummarizeInput } from "@/lib/engine-contract";
import type { AIChunk, AIRequest, SessionEvent } from "@/lib/types";
import { makeMode, makeResponse, makeSegment, makeSession, makeSettings } from "../../fixtures/helpers/builders";

function summarizeInput(overrides: Partial<SummarizeInput> = {}): SummarizeInput {
  return {
    session: makeSession({ id: "ses_sum" }),
    mode: makeMode({ id: "team-meeting", name: "Team Meeting", responseSchema: "meeting" }),
    transcript: [
      makeSegment({ id: "t1", speaker: "Speaker", source: "system", text: "Let's go with the phased migration." }),
      makeSegment({ id: "t2", speaker: "You", source: "microphone", text: "I'll own the rollout plan by Friday." }),
    ],
    responses: [makeResponse({ title: "Migration summary", content: "Postgres wins on cost." })],
    events: [
      {
        id: "e1",
        sessionId: "ses_sum",
        type: "decision_detected",
        title: "Decision detected",
        detail: "Phased Postgres migration",
        createdAt: "2026-09-07T09:14:10.000Z",
      },
    ],
    notes: [{ id: "n1", sessionId: "ses_sum", content: "Check licensing.", createdAt: "x", updatedAt: "x" }],
    settings: makeSettings(),
    ...overrides,
  };
}

function apiReturning(text: string) {
  const requests: AIRequest[] = [];
  return {
    requests,
    api: {
      ai: {
        stream: async (request: AIRequest, onChunk: (chunk: AIChunk) => void) => {
          requests.push(request);
          onChunk({ type: "delta", requestId: request.requestId, text });
          onChunk({ type: "completed", requestId: request.requestId, finishReason: "stop", totalMs: 50 });
        },
        cancel: async () => true,
      },
    },
  };
}

describe("generateSessionSummary", () => {
  const summaryJson = JSON.stringify({
    overview: "Standup about the database migration.",
    topics: ["database migration"],
    questions: ["Who owns the rollout?"],
    answers: ["Postgres wins on cost."],
    decisions: ["Phased Postgres migration"],
    actionItems: ["Sarah: rollout plan by Friday"],
    openItems: ["Licensing check"],
    improvements: ["Timebox the benchmark discussion"],
    sections: [{ title: "Deal notes", content: "n/a" }],
  });

  it("builds a summarization request with the summary schema and session material", async () => {
    const { api, requests } = apiReturning(summaryJson);
    const summary = await generateSessionSummary(summarizeInput(), {
      api,
      now: () => new Date("2026-09-07T10:00:00.000Z"),
      idGen: () => "fixed",
    });

    expect(requests).toHaveLength(1);
    const request = requests[0]!;
    expect(request.task).toBe("summarization");
    expect(request.sessionId).toBe("ses_sum");
    expect(request.outputSchema?.name).toBe("bluey_session_summary");
    const userPart = request.messages[1]?.content[0];
    const userText = userPart && "text" in userPart ? userPart.text : "";
    expect(userText).toContain("Speaker: Let's go with the phased migration.");
    expect(userText).toContain("Decision detected: Phased Postgres migration");
    expect(userText).toContain("Check licensing.");
    expect(userText).toContain("Decisions and action items are the priority");

    expect(summary.sessionId).toBe("ses_sum");
    expect(summary.modeId).toBe("team-meeting");
    expect(summary.decisions).toEqual(["Phased Postgres migration"]);
    expect(summary.actionItems).toEqual(["Sarah: rollout plan by Friday"]);
    expect(summary.sections).toHaveLength(1);
    expect(summary.id).toBe("sum_fixed");
    expect(summary.createdAt).toBe("2026-09-07T10:00:00.000Z");
  });

  it("degrades to an overview-only summary for malformed output", async () => {
    const { api } = apiReturning("The meeting was mostly about the migration.");
    const summary = await generateSessionSummary(summarizeInput(), { api });
    expect(summary.overview).toBe("The meeting was mostly about the migration.");
    expect(summary.decisions).toEqual([]);
  });

  it("reads null fields from strict-mode providers as empty", async () => {
    const { api } = apiReturning(
      JSON.stringify({ overview: "Short one.", topics: null, questions: null, answers: null, decisions: ["Ship"], actionItems: null, openItems: null, improvements: null, sections: null }),
    );
    const summary = await generateSessionSummary(summarizeInput(), { api });
    expect(summary.overview).toBe("Short one.");
    expect(summary.topics).toEqual([]);
    expect(summary.decisions).toEqual(["Ship"]);
    expect(summary.sections).toBeUndefined();
  });

  it("asks for a study guide in lecture mode", async () => {
    const { api, requests } = apiReturning(summaryJson);
    await generateSessionSummary(
      summarizeInput({ mode: makeMode({ id: "lecture", responseSchema: "lecture" }) }),
      { api },
    );
    const userPart = requests[0]?.messages[1]?.content[0];
    const userText = userPart && "text" in userPart ? userPart.text : "";
    expect(userText).toContain("Study guide");
  });

  it("throws a BlueyError when the stream fails", async () => {
    const api = {
      ai: {
        stream: async (request: AIRequest, onChunk: (chunk: AIChunk) => void) => {
          onChunk({
            type: "failed",
            requestId: request.requestId,
            error: { kind: "ai", code: "ai.no_provider", message: "no provider", recoverable: true },
          });
        },
        cancel: async () => true,
      },
    };
    await expect(generateSessionSummary(summarizeInput(), { api })).rejects.toMatchObject({
      code: "ai.no_provider",
    });
  });
});

describe("parseSummaryOutput / renderSummaryInput / schema", () => {
  it("parses fenced summary JSON", () => {
    const parsed = parseSummaryOutput('```json\n{"overview":"ok","topics":["a"]}\n```');
    expect(parsed.overview).toBe("ok");
    expect(parsed.topics).toEqual(["a"]);
  });

  it("renders only non-empty sections", () => {
    const text = renderSummaryInput(summarizeInput({ notes: [], responses: [], events: [] }));
    expect(text).toContain("### Transcript");
    expect(text).not.toContain("### User notes");
    expect(text).not.toContain("### Responses given");
  });

  it("summary schema is a strict named object", () => {
    const spec = summaryOutputSchema();
    expect(spec.name).toBe("bluey_session_summary");
    expect((spec.schema.properties as Record<string, unknown>).actionItems).toBeDefined();
  });
});

describe("buildTimeline", () => {
  const events: SessionEvent[] = [
    {
      id: "e1",
      sessionId: "s",
      type: "question_detected",
      title: "Question detected",
      detail: "Why us?",
      createdAt: "2026-09-07T09:14:05.000Z",
    },
    {
      id: "e2",
      sessionId: "s",
      type: "topic_change",
      title: "Topic change",
      createdAt: "2026-09-07T09:16:40.000Z",
    },
  ];

  it("groups events and responses by minute in order", () => {
    const responses = [
      makeResponse({ id: "r1", title: "Answer draft", prepared: true, createdAt: "2026-09-07T09:14:30.000Z" }),
    ];
    const timeline = buildTimeline(events, responses);
    expect(timeline).toHaveLength(2);
    expect(timeline[0]?.entries).toEqual(["Question detected: Why us?", "Response prepared: Answer draft"]);
    expect(timeline[1]?.entries).toEqual(["Topic change"]);
    expect(timeline[0]?.time).toMatch(/^\d{2}:\d{2}$/);
  });

  it("labels non-prepared responses as generated", () => {
    const timeline = buildTimeline([], [makeResponse({ title: undefined, createdAt: "2026-09-07T09:20:00.000Z" })]);
    expect(timeline[0]?.entries).toEqual(["Response generated"]);
  });
});

describe("composeSessionMarkdown", () => {
  it("composes a full markdown preview", () => {
    const markdown = composeSessionMarkdown({
      session: makeSession({ title: "Sprint 34 standup", startedAt: "2026-09-07T09:00:00.000Z" }),
      mode: makeMode({ name: "Team Meeting" }),
      summary: {
        id: "sum_1",
        sessionId: "ses_1",
        modeId: "team-meeting",
        overview: "Standup about the migration.",
        topics: ["migration"],
        questions: [],
        answers: [],
        decisions: ["Go with Postgres"],
        actionItems: ["Sarah: rollout by Friday"],
        openItems: [],
        improvements: [],
        sections: [{ title: "Extra", content: "Detail" }],
        createdAt: "2026-09-07T10:00:00.000Z",
      },
      timeline: [{ time: "09:14", at: "2026-09-07T09:14:05.000Z", entries: ["Question detected"] }],
      responses: [makeResponse({ title: "Benchmark recap", content: "Postgres wins." })],
      notes: [{ id: "n1", sessionId: "ses_1", content: "Check licensing.", createdAt: "x", updatedAt: "x" }],
    });

    expect(markdown).toContain("# Sprint 34 standup");
    expect(markdown).toContain("## Overview");
    expect(markdown).toContain("## Decisions");
    expect(markdown).toContain("- Go with Postgres");
    expect(markdown).toContain("## Timeline");
    expect(markdown).toContain("**09:14** — Question detected");
    expect(markdown).toContain("### Benchmark recap");
    expect(markdown).toContain("## Extra");
    expect(markdown).toContain("- Check licensing.");
  });

  it("omits empty sections", () => {
    const markdown = composeSessionMarkdown({ session: makeSession(), mode: makeMode() });
    expect(markdown).not.toContain("## Timeline");
    expect(markdown).not.toContain("## Responses");
  });
});
