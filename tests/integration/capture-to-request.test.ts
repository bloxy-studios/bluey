/**
 * Full ask pipeline: capture → context → request → streaming → optimized,
 * persisted response — through the real `bluey.*` api layer over a fake
 * transport.
 */

import { createResponseEngine } from "@/ai/engine";
import type { EnginePhase } from "@/lib/engine-contract";
import { setTransport } from "@/lib/tauri/transport";
import { eventBus } from "@/lib/tauri/event-bus";
import type { AIChunk, BlueyResponse, ContextSnapshot, SessionEvent } from "@/lib/types";
import { FakeTransport } from "../fixtures/helpers/fake-transport";
import { makeSession, makeSettings } from "../fixtures/helpers/builders";
import { loadFixture } from "../fixtures/helpers/fixtures";

const fixture = loadFixture("coding");

const structured = {
  responseType: "code",
  title: "Two Sum",
  content:
    "Use a hash map keyed by the needed complement.\n\n```python\ndef two_sum(nums, target):\n    seen = {}\n    for i, n in enumerate(nums):\n        if target - n in seen:\n            return [seen[target - n], i]\n        seen[n] = i\n```",
  sections: [
    { title: "Approach", content: "Single pass; store each value's index." },
    { title: "Solution", content: "See code block.", kind: "code", language: "python" },
    { title: "Complexity", content: "O(n) time, O(n) space." },
    { title: "Edge cases", content: "Duplicates and negative numbers work; assume exactly one answer." },
  ],
  code: {
    language: "python",
    code: "def two_sum(nums, target):\n    seen = {}\n    for i, n in enumerate(nums):\n        if target - n in seen:\n            return [seen[target - n], i]\n        seen[n] = i",
  },
  confidence: 0.9,
};

function codingScript() {
  const full = JSON.stringify(structured);
  const mid = full.indexOf("def two_sum"); // split INSIDE the fenced block
  const parts = [full.slice(0, mid), full.slice(mid)];
  return (request: { requestId: string }, emit: (chunk: AIChunk) => void): void => {
    emit({
      type: "started",
      requestId: request.requestId,
      selection: { providerId: "mock", providerKind: "mock", model: "mock-1", role: "reasoning", reason: "test" },
    });
    for (const part of parts) emit({ type: "delta", requestId: request.requestId, text: part });
    emit({ type: "usage", requestId: request.requestId, inputTokens: 900, outputTokens: 180 });
    emit({
      type: "completed",
      requestId: request.requestId,
      finishReason: "stop",
      totalMs: 1450,
      timeToFirstTokenMs: 210,
    });
  };
}

describe("capture → context → request → response", () => {
  let fake: FakeTransport;
  const saved: BlueyResponse[] = [];
  const events: SessionEvent[] = [];

  beforeEach(() => {
    fake = new FakeTransport();
    saved.length = 0;
    events.length = 0;
    fake.handle("context_build_snapshot", () => fixture.snapshot as ContextSnapshot);
    fake.handle("responses_save", ({ response }) => {
      saved.push(response);
      return response;
    });
    fake.handle("sessions_add_event", (args) => {
      const event: SessionEvent = {
        id: `evt_${events.length + 1}`,
        sessionId: args.sessionId,
        type: args.type,
        title: args.title,
        detail: args.detail,
        refs: args.refs,
        createdAt: "2026-09-07T09:10:00.000Z",
      };
      events.push(event);
      return event;
    });
    fake.handle("ai_cancel", () => true);
    fake.setAIScript(codingScript());
    setTransport(fake);
  });

  it("runs the whole pipeline and produces a persisted, optimized code response", async () => {
    const engine = createResponseEngine({ now: () => new Date("2026-09-07T09:09:00.000Z") });
    const phases: EnginePhase[] = [];
    const drafts: string[] = [];
    const contextUpdates: string[] = [];
    const offContext = eventBus.on("context.updated", ({ reason }) => contextUpdates.push(reason));

    let completed: BlueyResponse | null = null;
    const handle = engine.ask(
      {
        trigger: "shortcut_capture",
        captureScreen: true,
        mode: fixture.mode,
        session: makeSession({ id: "ses_int", modeId: fixture.mode.id }),
        settings: makeSettings(),
        previousResponses: [],
      },
      {
        onPhase: (phase) => phases.push(phase),
        onDraft: (response) => drafts.push(response.content),
        onComplete: (response) => {
          completed = response;
        },
      },
    );

    const result = await handle.done;
    offContext();

    // Snapshot was built with screen+OCR options for a capture trigger.
    const snapshotCalls = fake.callsFor("context_build_snapshot");
    expect(snapshotCalls).toHaveLength(1);
    expect(snapshotCalls[0]?.options).toMatchObject({
      includeScreen: true,
      includeOcr: true,
      includeAccessibility: true,
      includeTranscript: true,
      inlineImage: true,
      ocrLevel: "accurate",
    });

    // Coding mode declares no document needs — retrieval must be skipped.
    expect(fake.callsFor("documents_retrieve")).toHaveLength(0);

    // The AI request carries task, schema, budgeted context and prompt sections.
    const streamCalls = fake.callsFor("ai_stream");
    expect(streamCalls).toHaveLength(1);
    const request = streamCalls[0]!.request;
    expect(request.requestId).toBe(handle.requestId);
    expect(request.generation).toBe(1);
    expect(request.task).toBe("coding");
    expect(request.latencyBudget).toBe("balanced");
    expect(request.visionRequired).toBe(false);
    expect(request.outputSchema?.name).toBe("bluey_coding");
    expect(request.sessionId).toBe("ses_int");
    expect(request.contextTokens).toBeGreaterThan(0);
    expect(request.maxOutputTokens).toBeGreaterThanOrEqual(1600);

    const systemPart = request.messages[0]?.content[0];
    const systemText = systemPart && "text" in systemPart ? systemPart.text : "";
    expect(request.messages[0]?.role).toBe("system");
    expect(systemText).toContain("UNTRUSTED DATA");
    expect(systemText).toContain(fixture.mode.systemInstructions);

    const userPart = request.messages[1]?.content[0];
    const userText = userPart && "text" in userPart ? userPart.text : "";
    expect(userText).toContain("### On screen (OCR)");
    expect(userText).toContain("Two Sum");
    expect(userText).toContain("### Recent conversation (You / Speaker)");
    expect(userText).toContain("Task: Explain or solve what is on the screen");

    // Phases in order.
    expect(phases).toEqual(["capturing", "analyzing", "thinking", "streaming", "done"]);

    // Drafts grow monotonically and never expose an unterminated code fence.
    expect(drafts.length).toBeGreaterThan(0);
    for (let i = 1; i < drafts.length; i += 1) {
      expect(drafts[i]!.startsWith(drafts[i - 1]!)).toBe(true);
    }
    expect(drafts[0]).not.toContain("```");

    // Final response.
    expect(result).not.toBeNull();
    expect(completed).not.toBeNull();
    expect(result).toBe(completed);
    expect(result!.type).toBe("code");
    expect(result!.title).toBe("Two Sum");
    expect(result!.code?.language).toBe("python");
    expect(result!.code?.code).toContain("def two_sum");
    expect(result!.content).toContain("```python");
    expect(result!.sections?.map((s) => s.title)).toEqual(["Approach", "Solution", "Complexity", "Edge cases"]);
    expect(result!.confidence).toBe(0.9);
    expect(result!.metrics).toMatchObject({
      provider: "mock",
      model: "mock-1",
      inputTokens: 900,
      outputTokens: 180,
      totalMs: 1450,
      timeToFirstTokenMs: 210,
    });

    // Persistence + session event + local context event.
    expect(saved).toHaveLength(1);
    expect(saved[0]?.id).toBe(result!.id);
    expect(events).toHaveLength(1);
    expect(events[0]).toMatchObject({ type: "response_generated", sessionId: "ses_int" });
    expect(contextUpdates).toEqual(["response_generated"]);
  });

  it("reports errors through onError with a BlueyError and an error phase", async () => {
    fake.setAIScript((request, emit) => {
      emit({
        type: "failed",
        requestId: request.requestId,
        error: { kind: "ai", code: "ai.no_provider", message: "No provider configured", recoverable: true },
      });
    });
    const engine = createResponseEngine();
    const phases: EnginePhase[] = [];
    let errorCode: string | null = null;

    const handle = engine.ask(
      {
        trigger: "shortcut_capture",
        captureScreen: true,
        mode: fixture.mode,
        settings: makeSettings(),
      },
      {
        onPhase: (phase) => phases.push(phase),
        onError: (error) => {
          errorCode = error.code;
        },
      },
    );

    const result = await handle.done;
    expect(result).toBeNull();
    expect(errorCode).toBe("ai.no_provider");
    expect(phases[phases.length - 1]).toBe("error");
    expect(saved).toHaveLength(0);
  });
});
