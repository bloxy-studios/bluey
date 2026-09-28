/**
 * The HUD "Screen off" toggle (UX-002): with screen context off no ask —
 * ⌘↵, a screen-requiring mode, a typed question or a detected question —
 * captures the screen, runs OCR or reads the accessibility tree.
 */

import { createResponseEngine } from "@/ai/engine";
import { snapshotOptionsFor } from "@/context/snapshot";
import type { AskTrigger, EnginePhase } from "@/lib/engine-contract";
import { setTransport } from "@/lib/tauri/transport";
import type { AIChunk, ContextSnapshot, SnapshotOptions } from "@/lib/types";
import { FakeTransport } from "../../fixtures/helpers/fake-transport";
import { makeMode, makeSettings } from "../../fixtures/helpers/builders";

const screenMode = makeMode({ contextRequirements: ["screen", "accessibility", "transcript"] });
const settings = makeSettings();
const TRIGGERS: AskTrigger[] = ["typed", "shortcut_capture", "shortcut_generate", "detected_event", "assist"];

describe("snapshotOptionsFor with screen context off", () => {
  it("never includes the screen, OCR or the accessibility tree", () => {
    for (const trigger of TRIGGERS) {
      const options = snapshotOptionsFor({ mode: screenMode, settings, trigger, captureScreen: true, screenAllowed: false });
      expect(options).toMatchObject({ includeScreen: false, includeOcr: false, includeAccessibility: false });
      expect(options.inlineImage).toBe(false);
      expect(options.capture).toBeUndefined();
      expect(options.includeTranscript).toBe(true); // the transcript is not screen context
    }
  });

  it("still captures when the toggle is on or unset", () => {
    for (const screenAllowed of [true, undefined]) {
      const options = snapshotOptionsFor({ mode: screenMode, settings, trigger: "shortcut_capture", screenAllowed });
      expect(options.includeScreen).toBe(true);
      expect(options.includeAccessibility).toBe(true);
    }
  });
});

function answerScript(request: { requestId: string }, emit: (chunk: AIChunk) => void): void {
  emit({
    type: "started",
    requestId: request.requestId,
    selection: { providerId: "mock", providerKind: "mock", model: "mock-1", role: "default", reason: "test" },
  });
  emit({ type: "delta", requestId: request.requestId, text: "The deadline is Friday." });
  emit({ type: "completed", requestId: request.requestId, finishReason: "stop", totalMs: 300, timeToFirstTokenMs: 80 });
}

describe("engine with screen context off", () => {
  it("asks Rust for a snapshot without the screen on ⌘↵ in a screen mode", async () => {
    const fake = new FakeTransport();
    const requested: SnapshotOptions[] = [];
    fake.handle("context_build_snapshot", ({ options }) => {
      requested.push(options);
      return { timestamp: "2026-09-28T10:00:00.000Z" } as ContextSnapshot;
    });
    fake.handle("documents_retrieve", () => []);
    fake.handle("responses_save", ({ response }) => response);
    fake.handle("ai_cancel", () => true);
    fake.setAIScript(answerScript);
    setTransport(fake);

    const phases: EnginePhase[] = [];
    const handle = createResponseEngine().ask(
      {
        trigger: "shortcut_capture",
        instruction: "When is the deadline?",
        captureScreen: true,
        screenAllowed: false,
        mode: screenMode,
        settings,
      },
      { onPhase: (phase) => phases.push(phase) },
    );
    expect(await handle.done).not.toBeNull();

    expect(requested).toHaveLength(1);
    expect(requested[0]).toMatchObject({ includeScreen: false, includeOcr: false, includeAccessibility: false });
    expect(phases).not.toContain("capturing");
  });
});
