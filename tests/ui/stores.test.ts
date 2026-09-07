import { describe, expect, it, beforeEach } from "vitest";

import type { AppStatus } from "@/lib/types";
import { useAppStore } from "@/stores/appStore";
import { useChatStore } from "@/stores/chatStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { makeResponse, setupMockApp } from "./helpers";

function status(partial: Partial<AppStatus>): AppStatus {
  return { state: "ready", audioActive: false, modeId: "general", updatedAt: new Date().toISOString(), ...partial };
}

describe("appStore", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("loads the initial status from the backend", () => {
    expect(useAppStore.getState().status?.state).toBe("ready");
    expect(useAppStore.getState().status?.modeId).toBe("general");
  });

  it("mirrors app.state events", async () => {
    const mock = await setupMockApp();
    mock.emit("app.state", status({ state: "listening", audioActive: true }));
    expect(useAppStore.getState().status?.state).toBe("listening");
    expect(useAppStore.getState().status?.audioActive).toBe(true);
  });
});

describe("settingsStore", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("persists updates through the transport", async () => {
    const result = await useSettingsStore.getState().update({ general: { launchAtLogin: true } });
    expect(result?.general.launchAtLogin).toBe(true);
    expect(useSettingsStore.getState().settings?.general.launchAtLogin).toBe(true);
    // other sections untouched by the deep merge
    expect(useSettingsStore.getState().settings?.general.blueyName).toBe("Bluey");
    expect(useSettingsStore.getState().settings?.audio.source).toBe("both");
  });
});

describe("chatStore stale-draft protection", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("ignores drafts and completions from a superseded generation", () => {
    const chat = useChatStore.getState();
    const gen1 = chat.begin("first question");
    const gen2 = useChatStore.getState().begin("second question");
    expect(gen2).toBeGreaterThan(gen1);

    // Stale stream keeps writing — must be ignored.
    useChatStore.getState().applyDraft(gen1, makeResponse({ content: "STALE" }));
    let lastTurn = useChatStore.getState().turns.at(-1);
    expect(lastTurn?.response).toBeNull();

    useChatStore.getState().applyDraft(gen2, makeResponse({ content: "FRESH" }));
    lastTurn = useChatStore.getState().turns.at(-1);
    expect(lastTurn?.response?.content).toBe("FRESH");

    useChatStore.getState().complete(gen1, makeResponse({ content: "STALE DONE" }));
    lastTurn = useChatStore.getState().turns.at(-1);
    expect(lastTurn?.status).toBe("streaming");

    useChatStore.getState().complete(gen2, makeResponse({ content: "FRESH DONE" }));
    lastTurn = useChatStore.getState().turns.at(-1);
    expect(lastTurn?.status).toBe("done");
    expect(lastTurn?.response?.content).toBe("FRESH DONE");
  });

  it("supersedes the previous streaming turn when a new ask begins", () => {
    const gen1 = useChatStore.getState().begin("q1");
    expect(gen1).toBeGreaterThan(0);
    useChatStore.getState().begin("q2");
    const turns = useChatStore.getState().turns;
    expect(turns).toHaveLength(2);
    expect(turns[0]?.status).toBe("cancelled");
    expect(turns[1]?.status).toBe("streaming");
  });
});
