/**
 * Session → summary: mode-structured summarization request, tolerant parsing,
 * and persistence through `sessions_save_summary`.
 */

import { createResponseEngine } from "@/ai/engine";
import type { SummarizeInput } from "@/lib/engine-contract";
import { setTransport } from "@/lib/tauri/transport";
import type { AIChunk, SessionSummary } from "@/lib/types";
import { FakeTransport } from "../fixtures/helpers/fake-transport";
import { makeResponse, makeSession, makeSettings } from "../fixtures/helpers/builders";
import { loadFixture } from "../fixtures/helpers/fixtures";

const fixture = loadFixture("meeting");

const summaryJson = JSON.stringify({
  overview: "Standup focused on the database migration decision.",
  topics: ["database migration", "benchmarks"],
  questions: ["Who owns the rollout plan?"],
  answers: ["Postgres wins on cost."],
  decisions: ["Phased Postgres migration"],
  actionItems: ["Sarah: rollout plan by Friday"],
  openItems: ["Licensing review"],
  improvements: ["Share benchmarks before the meeting"],
  sections: [{ title: "Notes", content: "CockroachDB revisit in Q2." }],
});

function summarizeInput(): SummarizeInput {
  return {
    session: makeSession({ id: "ses_sum_int", modeId: fixture.mode.id }),
    mode: fixture.mode,
    transcript: fixture.transcript,
    responses: [makeResponse({ title: "Benchmark recap", content: "Postgres wins on cost." })],
    events: [
      {
        id: "e1",
        sessionId: "ses_sum_int",
        type: "decision_detected",
        title: "Decision detected",
        detail: "Phased Postgres migration",
        createdAt: "2026-09-07T09:14:10.000Z",
      },
    ],
    notes: [
      { id: "n1", sessionId: "ses_sum_int", content: "Check licensing.", createdAt: "x", updatedAt: "x" },
    ],
    settings: makeSettings(),
  };
}

function summaryScript(request: { requestId: string }, emit: (chunk: AIChunk) => void): void {
  emit({ type: "delta", requestId: request.requestId, text: summaryJson });
  emit({ type: "completed", requestId: request.requestId, finishReason: "stop", totalMs: 800 });
}

describe("summarizeSession", () => {
  it("streams a summarization request and persists the parsed summary", async () => {
    const fake = new FakeTransport();
    const savedSummaries: Array<Omit<SessionSummary, "id" | "createdAt">> = [];
    fake.handle("sessions_save_summary", ({ summary }) => {
      savedSummaries.push(summary);
      return { ...summary, id: "sum_backend", createdAt: "2026-09-07T10:00:00.000Z" };
    });
    fake.setAIScript(summaryScript);
    setTransport(fake);

    const engine = createResponseEngine();
    const summary = await engine.summarizeSession(summarizeInput());

    // Request shape.
    const request = fake.callsFor("ai_stream")[0]!.request;
    expect(request.task).toBe("summarization");
    expect(request.sessionId).toBe("ses_sum_int");
    expect(request.outputSchema?.name).toBe("bluey_session_summary");
    const userPart = request.messages[1]?.content[0];
    const userText = userPart && "text" in userPart ? userPart.text : "";
    expect(userText).toContain("let's go with the phased Postgres migration");
    expect(userText).toContain("Decisions and action items are the priority");

    // Persistence used the backend-assigned identity.
    expect(savedSummaries).toHaveLength(1);
    expect(savedSummaries[0]).not.toHaveProperty("id");
    expect(savedSummaries[0]?.modeId).toBe("team-meeting");
    expect(summary.id).toBe("sum_backend");
    expect(summary.decisions).toEqual(["Phased Postgres migration"]);
    expect(summary.actionItems).toEqual(["Sarah: rollout plan by Friday"]);
    expect(summary.sections).toEqual([{ title: "Notes", content: "CockroachDB revisit in Q2." }]);
  });

  it("still returns the summary when persistence fails", async () => {
    const fake = new FakeTransport();
    fake.handle("sessions_save_summary", () => {
      throw new Error("db locked");
    });
    fake.setAIScript(summaryScript);
    setTransport(fake);

    const engine = createResponseEngine();
    const summary = await engine.summarizeSession(summarizeInput());
    expect(summary.id.startsWith("sum_")).toBe(true);
    expect(summary.overview).toContain("database migration");
  });

  it("degrades to an overview-only summary when the model returns prose", async () => {
    const fake = new FakeTransport();
    fake.handle("sessions_save_summary", ({ summary }) => ({
      ...summary,
      id: "sum_backend",
      createdAt: "2026-09-07T10:00:00.000Z",
    }));
    fake.setAIScript((request, emit) => {
      emit({ type: "delta", requestId: request.requestId, text: "We mostly discussed the migration." });
      emit({ type: "completed", requestId: request.requestId, finishReason: "stop", totalMs: 100 });
    });
    setTransport(fake);

    const engine = createResponseEngine();
    const summary = await engine.summarizeSession(summarizeInput());
    expect(summary.overview).toBe("We mostly discussed the migration.");
    expect(summary.decisions).toEqual([]);
  });
});
