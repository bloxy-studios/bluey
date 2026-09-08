/**
 * Privacy → Cloud AI off: the engine must not capture, retrieve or call a model.
 */

import { beforeEach, describe, expect, it } from "vitest";

import { CLOUD_AI_DISABLED_CODE } from "@/ai/cloud-gate";
import { createResponseEngine } from "@/ai/engine";
import { setTransport } from "@/lib/tauri/transport";
import type { BlueyError, ContextSnapshot, TranscriptSegment } from "@/lib/types";
import { makeMode, makeSettings } from "../fixtures/helpers/builders";
import { FakeTransport } from "../fixtures/helpers/fake-transport";
import { loadFixture } from "../fixtures/helpers/fixtures";

const segment: TranscriptSegment = {
  id: "seg-1",
  source: "system",
  speaker: "Interviewer",
  text: "Can you walk me through your résumé?",
  startTime: 0,
  endTime: 2000,
  finalized: true,
  createdAt: "2026-09-07T09:00:00.000Z",
};

describe("cloud AI gate", () => {
  let fake: FakeTransport;

  beforeEach(() => {
    fake = new FakeTransport();
    setTransport(fake);
  });

  it("ask fails fast with a configuration error that opens Privacy settings", async () => {
    const engine = createResponseEngine();
    const errors: BlueyError[] = [];
    const phases: string[] = [];
    const handle = engine.ask(
      {
        trigger: "typed",
        instruction: "What does this error mean?",
        captureScreen: true,
        mode: makeMode(),
        settings: makeSettings({ privacy: { cloudAiEnabled: false } }),
      },
      { onError: (error) => errors.push(error), onPhase: (phase) => phases.push(phase) },
    );

    expect(await handle.done).toBeNull();
    expect(errors).toHaveLength(1);
    expect(errors[0]?.code).toBe(CLOUD_AI_DISABLED_CODE);
    expect(errors[0]?.kind).toBe("configuration");
    expect(errors[0]?.recovery).toEqual({ type: "open_settings", tab: "privacy" });
    expect(phases).toEqual(["error"]);

    const commands = fake.calls.map((call) => call.command);
    expect(commands).not.toContain("ai_stream");
    expect(commands).not.toContain("context_build_snapshot");
    expect(commands).not.toContain("documents_retrieve");
  });

  it("prepare returns null and classify stays heuristic-only", async () => {
    const engine = createResponseEngine();
    const settings = makeSettings({
      privacy: { cloudAiEnabled: false },
      ai: {
        proactivePreparation: true,
        models: {
          default: { providerId: "p", model: "m" },
          fast: { providerId: "p", model: "fast" },
          reasoning: null,
          vision: null,
          research: null,
          transcription: null,
          embedding: null,
        },
      },
    });

    const prepared = await engine.prepare({
      trigger: "detected_event",
      captureScreen: false,
      mode: makeMode(),
      settings,
      detectedEvent: {
        id: "det-1",
        type: "question",
        confidence: 0.9,
        requiresResponse: true,
        text: segment.text,
        segmentIds: [segment.id],
        detectedAt: segment.createdAt,
      },
    });
    expect(prepared).toBeNull();

    const event = await engine.classify({ segment, recent: [], mode: makeMode(), settings });
    expect(event?.type).toBe("question"); // heuristics still run locally
    expect(fake.calls.map((call) => call.command)).not.toContain("ai_stream");
  });

  it("lets everything through when Cloud AI is on", async () => {
    const engine = createResponseEngine();
    fake.handle("context_build_snapshot", () => loadFixture("coding").snapshot as ContextSnapshot);
    fake.handle("responses_save", ({ response }) => response);
    fake.handle("ai_cancel", () => true);
    const handle = engine.ask(
      {
        trigger: "typed",
        instruction: "hello",
        captureScreen: false,
        mode: makeMode(),
        settings: makeSettings({ privacy: { cloudAiEnabled: true } }),
      },
      {},
    );
    const response = await handle.done;
    expect(response?.content).toContain("mock answer");
    expect(fake.calls.map((call) => call.command)).toContain("ai_stream");
  });
});
