import { describe, expect, it } from "vitest";

import { COMMAND_NAMES } from "@/lib/tauri/commands";
import { MockTransport } from "@/lib/tauri/mock";
import type { AIChunk, AIRequest, DetectedEvent, TranscriptSegment } from "@/lib/types";

function makeRequest(): AIRequest {
  return {
    requestId: "req-test",
    generation: 1,
    task: "answer",
    latencyBudget: "fast",
    reasoning: "none",
    visionRequired: false,
    contextTokens: 100,
    messages: [{ role: "user", content: [{ type: "text", text: "Solve the problem on my screen" }] }],
    createdAt: new Date().toISOString(),
  };
}

describe("MockTransport", () => {
  it("implements every command in CommandMap", () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    const handlers = (mock as unknown as { handlers: Record<string, unknown> }).handlers;
    for (const name of COMMAND_NAMES) {
      expect(typeof handlers[name], `handler for ${name}`).toBe("function");
    }
  });

  it("streams a canned markdown answer with a fenced code block over the channel", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    const channel = mock.createChannel<AIChunk>();
    const chunks: AIChunk[] = [];
    channel.onMessage((chunk) => chunks.push(chunk));

    await mock.invoke("ai_stream", { request: makeRequest(), onChunk: channel.raw });

    expect(chunks[0]?.type).toBe("started");
    const text = chunks
      .filter((c): c is Extract<AIChunk, { type: "delta" }> => c.type === "delta")
      .map((c) => c.text)
      .join("");
    expect(text).toContain("```python");
    expect(text).toContain("def two_sum");
    const last = chunks.at(-1);
    expect(last?.type).toBe("completed");
    if (last?.type === "completed") expect(last.finishReason).toBe("stop");
  });

  it("cancels an in-flight stream", async () => {
    const mock = new MockTransport({ streamDelayMs: 1, levelTicks: false });
    const channel = mock.createChannel<AIChunk>();
    const chunks: AIChunk[] = [];
    channel.onMessage((chunk) => {
      chunks.push(chunk);
      if (chunks.length === 3) void mock.invoke("ai_cancel", { requestId: "req-test" });
    });
    await mock.invoke("ai_stream", { request: makeRequest(), onChunk: channel.raw });
    const last = chunks.at(-1);
    expect(last?.type).toBe("completed");
    if (last?.type === "completed") expect(last.finishReason).toBe("cancelled");
  });

  it("dev_simulate question emits transcript.final and question.detected", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    const finals: TranscriptSegment[] = [];
    const detected: DetectedEvent[] = [];
    await mock.listen("transcript.final", (s) => finals.push(s));
    await mock.listen("question.detected", (d) => detected.push(d));

    await mock.invoke("dev_simulate", {
      simulation: { type: "question", text: "What is your greatest weakness?", speaker: "Interviewer" },
    });

    expect(finals).toHaveLength(1);
    expect(finals[0]?.text).toContain("greatest weakness");
    expect(detected).toHaveLength(1);
    expect(detected[0]?.type).toBe("question");
    expect(detected[0]?.requiresResponse).toBe(true);
  });

  it("audio_start flips the app state to listening and audio_stop back", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    const startStatus = await mock.invoke("audio_start", {});
    expect(startStatus.state).toBe("running");
    const appAfterStart = await mock.invoke("app_get_status", undefined);
    expect(appAfterStart.audioActive).toBe(true);
    expect(appAfterStart.state).toBe("listening");

    await mock.invoke("audio_stop", undefined);
    const appAfterStop = await mock.invoke("app_get_status", undefined);
    expect(appAfterStop.audioActive).toBe(false);
    expect(appAfterStop.state).toBe("ready");
  });

  it("seeds the ten built-in modes with instructions and groups", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    const modes = await mock.invoke("modes_list", undefined);
    expect(modes).toHaveLength(10);
    expect(modes.map((m) => m.id)).toContain("coding-interview");
    const interview = modes.find((m) => m.id === "interview");
    expect(interview?.group).toBe("Looking for work");
    expect(interview?.systemInstructions).toContain("candidate in a job interview");
  });

  it("seeds Gemini as the first provider and applies its presets per role", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    const settings = await mock.invoke("settings_get", undefined);
    expect(settings.ai.providers[0]).toMatchObject({ id: "gemini", kind: "google_gemini", hasApiKey: true });
    expect(settings.ai.bootstrapProvider).toBe("gemini");
    expect(settings.ai.embeddingDimensions).toBe(768);
    expect(settings.ai.researchBackend).toBe("gemini");

    // Fill-only leaves the seeded Foundry assignments alone; overwrite moves them to Gemini.
    const filled = await mock.invoke("ai_apply_provider_presets", { providerId: "gemini", overwrite: false });
    expect(filled.ai.models.default?.providerId).toBe("azure-foundry");
    const overwritten = await mock.invoke("ai_apply_provider_presets", {
      providerId: "gemini",
      overwrite: true,
    });
    expect(overwritten.ai.models.default).toEqual({ providerId: "gemini", model: "gemini-3.8-flash" });
    expect(overwritten.ai.models.fast).toEqual({ providerId: "gemini", model: "gemini-3.5-flash-lite" });
    expect(overwritten.ai.models.embedding).toEqual({ providerId: "gemini", model: "gemini-embedding-2" });
    expect(overwritten.ai.models.transcription).toEqual({
      providerId: "gemini",
      model: "gemini-3.5-transcribe",
    });
    await expect(
      mock.invoke("ai_apply_provider_presets", { providerId: "nope", overwrite: false }),
    ).rejects.toMatchObject({
      code: "config.unknown_provider",
    });
  });

  it("ai_list_models narrows the Gemini catalogue per role", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    const all = await mock.invoke("ai_list_models", { providerId: "gemini" });
    expect(all).toContain("gemini-3.8-flash");
    expect(await mock.invoke("ai_list_models", { providerId: "gemini", role: "embedding" })).toEqual([
      "gemini-embedding-2",
      "gemini-embedding-001",
    ]);
    expect(await mock.invoke("ai_list_models", { providerId: "gemini", role: "transcription" })).toEqual([
      "gemini-3.5-transcribe",
      "gemini-3.5-transcribe-live",
    ]);
    const text = await mock.invoke("ai_list_models", { providerId: "gemini", role: "default" });
    expect(text).toEqual(["gemini-3.8-flash", "gemini-3.5-flash-lite", "gemini-3.5-flash"]);
  });

  it("shortcuts_check_conflict flags bluey and system conflicts", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    const system = await mock.invoke("shortcuts_check_conflict", { accelerator: "CmdOrCtrl+Q" });
    expect(system?.conflictsWith).toBe("system");
    const bluey = await mock.invoke("shortcuts_check_conflict", {
      accelerator: "CmdOrCtrl+R",
      ignoreId: "toggle_panel",
    });
    expect(bluey?.conflictsWith).toBe("bluey");
    const ok = await mock.invoke("shortcuts_check_conflict", { accelerator: "CmdOrCtrl+Alt+P" });
    expect(ok).toBeNull();
  });

  it("context_build_snapshot returns OCR text of a coding problem", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    const snapshot = await mock.invoke("context_build_snapshot", {
      options: {
        includeScreen: true,
        includeOcr: true,
        includeAccessibility: false,
        includeTranscript: false,
      },
    });
    expect(snapshot.ocr?.text).toContain("Two Sum");
    expect(snapshot.screen?.width).toBeGreaterThan(0);
  });
});
