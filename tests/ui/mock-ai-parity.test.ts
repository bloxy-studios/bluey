// The mock's `ai_*` handlers must stay as strict as Rust's `AiManager` (TEST-003):
// supersede by (session, scope, generation), background work never moves the app
// state, only a primary answer's failure lands in Error, and Cloud AI off refuses.
import { describe, expect, it } from "vitest";

import { MockTransport } from "@/lib/tauri/mock";
import type { AIChunk, AIRequest } from "@/lib/types";

function request(id: string, patch: Partial<AIRequest> = {}): AIRequest {
  return {
    requestId: id,
    generation: 1,
    task: "answer",
    latencyBudget: "fast",
    reasoning: "none",
    visionRequired: false,
    contextTokens: 100,
    messages: [{ role: "user", content: [{ type: "text", text: "What is a closure?" }] }],
    createdAt: new Date().toISOString(),
    sessionId: "ses_1",
    ...patch,
  };
}

/** Start a stream and resolve with its chunks once it ends. */
function stream(mock: MockTransport, req: AIRequest): { chunks: AIChunk[]; done: Promise<void> } {
  const channel = mock.createChannel<AIChunk>();
  const chunks: AIChunk[] = [];
  channel.onMessage((chunk) => chunks.push(chunk));
  const done = mock.invoke("ai_stream", { request: req, onChunk: channel.raw });
  return { chunks, done };
}

function finishReason(chunks: AIChunk[]): string | undefined {
  const last = chunks.at(-1);
  return last?.type === "completed" ? last.finishReason : last?.type;
}

describe("MockTransport AI parity with AiManager", () => {
  it("supersedes only an older generation of the same session and scope", async () => {
    const mock = new MockTransport({ streamDelayMs: 1, levelTicks: false });
    const live = stream(mock, request("live", { scope: "live", background: true, generation: 1 }));
    const manual = stream(mock, request("manual", { scope: "chat", generation: 2 }));
    const correction = stream(mock, request("manual-2", { scope: "chat", generation: 3 }));
    await Promise.all([live.done, manual.done, correction.done]);

    expect(finishReason(live.chunks)).toBe("stop");
    expect(finishReason(manual.chunks)).toBe("cancelled");
    expect(finishReason(correction.chunks)).toBe("stop");
  });

  it("keeps background work out of the app state, even when it fails", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    const states: string[] = [];
    await mock.listen("app.state", (payload) => states.push(payload.state));

    await stream(mock, request("prep", { scope: "live", background: true })).done;
    await mock.invoke("dev_simulate", { simulation: { type: "ai_failure" } });
    const failed = stream(mock, request("prep-2", { scope: "live", background: true, generation: 2 }));
    await failed.done;

    expect(failed.chunks.at(-1)?.type).toBe("failed");
    expect(states).toEqual([]);
    expect((await mock.invoke("app_get_status", undefined)).state).toBe("ready");
  });

  it("lands a primary failure in Error and leaves it on the next answer", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    await mock.invoke("dev_simulate", { simulation: { type: "ai_failure" } });
    await stream(mock, request("ask-1")).done;
    expect((await mock.invoke("app_get_status", undefined)).state).toBe("error");

    const states: string[] = [];
    await mock.listen("app.state", (payload) => states.push(payload.state));
    await stream(mock, request("ask-2", { generation: 2 })).done;
    expect(states).toEqual(["thinking", "response_ready"]);
  });

  it("only cancels streams still in flight, like AiManager::cancel", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    await stream(mock, request("done")).done;
    expect(await mock.invoke("ai_cancel", { requestId: "done" })).toBe(false);
    expect(await mock.invoke("ai_cancel_all", undefined)).toBe(0);
  });

  it("refuses every model call when Privacy → Cloud AI is off (SEC-003)", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    const settings = await mock.invoke("settings_get", undefined);
    await mock.invoke("settings_update", {
      patch: { privacy: { ...settings.privacy, cloudAiEnabled: false } },
    });

    const refused = stream(mock, request("ask"));
    await expect(refused.done).rejects.toMatchObject({ code: "privacy.cloud_ai_disabled" });
    await expect(mock.invoke("ai_embed", { texts: ["hello"], purpose: "query" })).rejects.toMatchObject({
      code: "privacy.cloud_ai_disabled",
    });
    expect((await mock.invoke("app_get_status", undefined)).state).toBe("error");
  });
});
