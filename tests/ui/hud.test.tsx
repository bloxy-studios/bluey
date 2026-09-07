import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { HudPanel } from "@/features/hud/HudPanel";
import type { AppStatus } from "@/lib/types";
import { setEngine } from "@/stores/engine";
import { useChatStore } from "@/stores/chatStore";
import { FakeEngine, makeResponse, setupMockApp } from "./helpers";

function renderHud() {
  return render(
    <TooltipProvider>
      <HudPanel />
    </TooltipProvider>,
  );
}

function status(partial: Partial<AppStatus>): AppStatus {
  return { state: "ready", audioActive: false, modeId: "general", updatedAt: new Date().toISOString(), ...partial };
}

describe("HudPanel", () => {
  let engine: FakeEngine;

  beforeEach(async () => {
    await setupMockApp();
    engine = new FakeEngine();
    setEngine(engine);
  });

  it("renders the idle layout with input, pill and History control", async () => {
    renderHud();
    expect(screen.getByPlaceholderText("Ask anything about your screen")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("Bluey · General")).toBeInTheDocument());
    expect(screen.getByText("History")).toBeInTheDocument();
  });

  it("shows the listening pill when audio is active", async () => {
    const mock = await setupMockApp();
    setEngine(engine);
    renderHud();
    mock.emit("app.state", status({ state: "listening", audioActive: true }));
    await waitFor(() => expect(screen.getByText("Listening")).toBeInTheDocument());
  });

  it("submits a typed question to the engine and streams the draft", async () => {
    const user = userEvent.setup();
    renderHud();

    const input = screen.getByPlaceholderText("Ask anything about your screen");
    await user.type(input, "What is this error?{Enter}");

    expect(engine.asks).toHaveLength(1);
    expect(engine.asks[0]?.trigger).toBe("typed");
    expect(engine.asks[0]?.instruction).toBe("What is this error?");
    expect(engine.asks[0]?.captureScreen).toBe(true);
    expect(engine.asks[0]?.mode.id).toBe("general");

    // Expanded layout: the prompt pill + follow-up header appear.
    expect(screen.getByText("What is this error?")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Ask follow-up")).toBeInTheDocument();

    engine.emitPhase("streaming");
    engine.emitDraft(makeResponse({ content: "It means the variable is **undefined**" }));
    await waitFor(() => expect(screen.getByText(/undefined/)).toBeInTheDocument());

    engine.complete(makeResponse({ content: "It means the variable is **undefined**. Fix the import." }));
    await waitFor(() => expect(screen.getByLabelText("Copy answer")).toBeInTheDocument());
    // New Chat control with ⌘R appears once a chat exists
    expect(screen.getByText("New Chat")).toBeInTheDocument();
  });

  it("Escape cancels a streaming request", async () => {
    const user = userEvent.setup();
    renderHud();
    await user.type(screen.getByPlaceholderText("Ask anything about your screen"), "hello{Enter}");
    expect(useChatStore.getState().turns.at(-1)?.status).toBe("streaming");

    await user.keyboard("{Escape}");
    await waitFor(() => expect(engine.cancelled).toBe(true));
    expect(useChatStore.getState().turns.at(-1)?.status).toBe("cancelled");
  });

  it("⌘↵ with an empty input asks with the Assist label and screen capture", async () => {
    const user = userEvent.setup();
    renderHud();
    await user.keyboard("{Meta>}{Enter}{/Meta}");
    expect(engine.asks).toHaveLength(1);
    expect(engine.asks[0]?.trigger).toBe("shortcut_capture");
    expect(engine.asks[0]?.captureScreen).toBe(true);
    expect(screen.getByText("Assist")).toBeInTheDocument();
  });

  it("shows a prepared-response hint and takes it with ⌘⇧↵", async () => {
    const mock = await setupMockApp();
    setEngine(engine);
    const prepared = makeResponse({ id: "prep-1", content: "Prepared answer", prompt: "Tell me about yourself", prepared: true });
    engine.preparedQueue.push(prepared);
    renderHud();

    mock.emit("response.prepared", prepared);
    await waitFor(() => expect(screen.getByText(/Bluey has a suggestion/)).toBeInTheDocument());

    const user = userEvent.setup();
    await user.keyboard("{Meta>}{Shift>}{Enter}{/Shift}{/Meta}");
    await waitFor(() => expect(screen.getByText("Prepared answer")).toBeInTheDocument());
    expect(engine.asks).toHaveLength(0); // took the prepared response, no new ask
  });
});
