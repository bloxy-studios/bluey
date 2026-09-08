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

    // Every role is seeded with Gemini's recommended model (as `ai_apply_provider_presets` leaves it).
    expect(settings.ai.models.default).toEqual({ providerId: "gemini", model: "gemini-3.8-flash" });
    expect(settings.ai.models.fast).toEqual({ providerId: "gemini", model: "gemini-3.5-flash-lite" });
    expect(settings.ai.models.embedding).toEqual({ providerId: "gemini", model: "gemini-embedding-2" });
    expect(settings.ai.models.transcription).toEqual({
      providerId: "gemini",
      model: "gemini-3.5-transcribe",
    });
    expect(settings.general.outputLanguage).toBe("en");

    // Fill-only leaves the seeded Gemini assignments alone; overwrite moves them to Foundry.
    const filled = await mock.invoke("ai_apply_provider_presets", {
      providerId: "azure-foundry",
      overwrite: false,
    });
    expect(filled.ai.models.default?.providerId).toBe("gemini");
    const overwritten = await mock.invoke("ai_apply_provider_presets", {
      providerId: "azure-foundry",
      overwrite: true,
    });
    expect(overwritten.ai.models.default).toEqual({ providerId: "azure-foundry", model: "gpt-5.6-terra" });
    expect(overwritten.ai.models.transcription).toEqual({
      providerId: "azure-foundry",
      model: "MAI-Transcribe-1.5",
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

  it("ai_test_connection follows the Rust contract: throws for unknown providers, picks the provider's model", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    await expect(mock.invoke("ai_test_connection", { providerId: "nope" })).rejects.toMatchObject({
      kind: "configuration",
      code: "config.unknown_provider",
    });

    // The role assignment's model when one points at the provider …
    expect(await mock.invoke("ai_test_connection", { providerId: "gemini" })).toMatchObject({
      ok: true,
      model: "gemini-3.8-flash",
    });
    // … the preset default otherwise; a missing key is a result, not an exception.
    const keyless = await mock.invoke("ai_test_connection", { providerId: "anthropic" });
    expect(keyless.ok).toBe(false);
    expect(keyless.model).toBe("claude-sonnet-5");
    expect(keyless.error).toMatchObject({ kind: "configuration", code: "config.missing_key" });

    // `dev_simulate ai_failure` fails the next test too (one-shot).
    await mock.invoke("dev_simulate", { simulation: { type: "ai_failure", code: "config.api_key_invalid" } });
    expect((await mock.invoke("ai_test_connection", { providerId: "gemini" })).error?.code).toBe(
      "config.api_key_invalid",
    );
    expect((await mock.invoke("ai_test_connection", { providerId: "gemini" })).ok).toBe(true);
  });

  it("ai_transcribe_file honours transcript storage and sessions_delete purges the segments", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    const imported = await mock.invoke("ai_transcribe_file", {
      path: "/tmp/standup.wav",
      diarization: true,
      wordTimestamps: true,
    });
    expect(imported.stored).toBe(true);
    expect(await mock.invoke("transcript_list", { sessionId: imported.session.id })).toHaveLength(4);
    expect((await mock.invoke("sessions_get", { id: imported.session.id })).transcriptSegmentCount).toBe(4);

    await mock.invoke("sessions_delete", { id: imported.session.id });
    expect(await mock.invoke("transcript_list", { sessionId: imported.session.id })).toHaveLength(0);

    await mock.invoke("settings_update", { patch: { privacy: { storeTranscripts: false } } });
    const unsaved = await mock.invoke("ai_transcribe_file", {
      path: "/tmp/retro.mp3",
      diarization: false,
      wordTimestamps: false,
    });
    expect(unsaved.stored).toBe(false);
    expect(unsaved.segments).toHaveLength(4); // returned to the caller …
    expect(await mock.invoke("transcript_list", { sessionId: unsaved.session.id })).toHaveLength(0); // … not kept
  });

  it("audio_pick_recording can be scripted to cancel, one pick at a time", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    mock.nextPickedRecording = null;
    expect(await mock.invoke("audio_pick_recording", undefined)).toBeNull();
    expect(await mock.invoke("audio_pick_recording", undefined)).toBe("/Users/jordan/Recordings/standup.wav");
    mock.nextPickedRecording = "/tmp/other.flac";
    expect(await mock.invoke("audio_pick_recording", undefined)).toBe("/tmp/other.flac");
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
