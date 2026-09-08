import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { HudPanel } from "@/features/hud/HudPanel";
import { endSession, pauseSession, resumeSession } from "@/features/hud/session-actions";
import { bluey } from "@/lib/tauri/api";
import type { AppStatus } from "@/lib/types";
import { useAppStore } from "@/stores/appStore";
import { setEngine } from "@/stores/engine";
import { useChatStore } from "@/stores/chatStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { FakeEngine, ProactiveFakeEngine, makeResponse, makeSegment, setupMockApp } from "./helpers";

function renderHud() {
  return render(
    <TooltipProvider>
      <HudPanel />
    </TooltipProvider>,
  );
}

function status(partial: Partial<AppStatus>): AppStatus {
  return {
    state: "ready",
    audioActive: false,
    modeId: "general",
    updatedAt: new Date().toISOString(),
    ...partial,
  };
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
    const prepared = makeResponse({
      id: "prep-1",
      content: "Prepared answer",
      prompt: "Tell me about yourself",
      prepared: true,
    });
    engine.preparedQueue.push(prepared);
    renderHud();

    mock.emit("response.prepared", prepared);
    await waitFor(() => expect(screen.getByText(/Bluey has a suggestion/)).toBeInTheDocument());

    const user = userEvent.setup();
    await user.keyboard("{Meta>}{Shift>}{Enter}{/Shift}{/Meta}");
    await waitFor(() => expect(screen.getByText("Prepared answer")).toBeInTheDocument());
    expect(engine.asks).toHaveLength(0); // took the prepared response, no new ask
  });

  it("prepares a response for a detected question and ⌘⇧↵ takes that exact one", async () => {
    const mock = await setupMockApp();
    const proactive = new ProactiveFakeEngine();
    setEngine(proactive);
    await useSettingsStore.getState().update({ ai: { proactivePreparation: true } });
    renderHud();

    mock.emit("transcript.final", makeSegment({ text: "Why do you want to work here?" }));
    await waitFor(() => expect(screen.getByText(/Bluey has a suggestion/)).toBeInTheDocument());

    const user = userEvent.setup();
    await user.keyboard("{Meta>}{Shift>}{Enter}{/Shift}{/Meta}");
    await waitFor(() => expect(screen.getByText("Prepared for det-1")).toBeInTheDocument());
    expect(proactive.takeCalls[0]).toBe("det-1");
    expect(proactive.asks).toHaveLength(0);
  });

  it("shows the live transcript strip while listening and collapses it to one line", async () => {
    const mock = await setupMockApp();
    setEngine(engine);
    renderHud();
    expect(screen.queryByLabelText("Live transcript")).not.toBeInTheDocument();

    mock.emit("app.state", status({ state: "listening", audioActive: true }));
    await waitFor(() => expect(screen.getByLabelText("Live transcript")).toBeInTheDocument());
    expect(screen.getByText("Waiting for speech…")).toBeInTheDocument();

    // A confident speaker label from the pipeline beats the mode heuristic ("Speaker" in General).
    mock.emit(
      "transcript.final",
      makeSegment({ text: "Walk me through your background.", speakerConfidence: 0.8 }),
    );
    mock.emit("transcript.final", makeSegment({ text: "And what are you looking for next?" }));
    mock.emit("transcript.partial", makeSegment({ text: "We are hiring for", finalized: false }));
    await waitFor(() => expect(screen.getByText("And what are you looking for next?")).toBeInTheDocument());
    expect(screen.getByText("Walk me through your background.")).toBeInTheDocument();
    expect(screen.getByText("We are hiring for")).toBeInTheDocument();
    expect(screen.getByText("Interviewer")).toBeInTheDocument();
    expect(screen.getAllByText("Speaker")).toHaveLength(2);

    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Collapse transcript" }));
    await waitFor(() =>
      expect(screen.queryByText("Walk me through your background.")).not.toBeInTheDocument(),
    );
    expect(screen.getByText("We are hiring for")).toBeInTheDocument(); // the newest line survives
    expect(screen.getByRole("button", { name: "Expand transcript" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );

    mock.emit("app.state", status({ state: "ready", audioActive: false }));
    await waitFor(() => expect(screen.queryByLabelText("Live transcript")).not.toBeInTheDocument());
  });

  it("surfaces the backend error in the pill with its recovery and dismisses via recover", async () => {
    const mock = await setupMockApp();
    setEngine(engine);
    renderHud();

    mock.emit(
      "app.state",
      status({
        state: "error",
        error: {
          kind: "configuration",
          code: "config.missing_provider",
          message: "No AI provider configured",
          recoverable: true,
          recovery: { type: "open_settings", tab: "ai" },
        },
      }),
    );
    await waitFor(() => expect(screen.getByText("Setup needed")).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "Open Settings" })).toBeInTheDocument();

    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Dismiss error" }));
    await waitFor(() => expect(useAppStore.getState().status?.state).toBe("ready"));
    expect(screen.queryByText("Setup needed")).not.toBeInTheDocument();
  });

  it("shows deep-research progress while thinking and lets the user skip it", async () => {
    const mock = await setupMockApp();
    setEngine(engine);
    const user = userEvent.setup();
    renderHud();
    await user.type(
      screen.getByPlaceholderText("Ask anything about your screen"),
      "Compare Rust web frameworks{Enter}",
    );
    engine.emitPhase("thinking");
    await screen.findByText("Thinking…");

    mock.emit("research.event", { type: "started", jobId: "job-1" });
    await screen.findByText("Researching");
    mock.emit("research.event", {
      type: "tool_call",
      jobId: "job-1",
      tool: "exa_search",
      input: { query: "rust" },
    });
    await screen.findByText(/Researching · Searching the web… \(1 lookup\)/);

    await user.click(screen.getByRole("button", { name: "Skip research" }));
    await screen.findByText(/Skipping research…/);
    expect(screen.getByRole("button", { name: "Skip research" })).toBeDisabled();

    mock.emit("research.event", {
      type: "failed",
      jobId: "job-1",
      error: { kind: "cancelled", code: "cancelled", message: "cancelled", recoverable: false },
    });
    await screen.findByText("Thinking…");
    expect(screen.queryByText("Researching")).not.toBeInTheDocument();
  });

  it("toolbar reflects the active session and its controls pause, resume and end it", async () => {
    // (The Radix menu itself is not driven here: opening it takes seconds under jsdom.)
    await bluey.audio.start();
    await bluey.session.start({ title: "Panel interview" });
    renderHud();
    await screen.findByRole("button", { name: "Session: Panel interview" });

    expect(await pauseSession()).toBe(true);
    await waitFor(() => expect(useSessionStore.getState().active?.status).toBe("paused"));
    expect((await bluey.audio.getStatus()).state).toBe("paused");

    expect(await resumeSession()).toBe(true);
    await waitFor(() => expect(useSessionStore.getState().active?.status).toBe("active"));
    expect((await bluey.audio.getStatus()).state).toBe("running");

    expect(await endSession()).toBe(true);
    await waitFor(() => expect(useSessionStore.getState().active).toBeNull());
    expect(await screen.findByRole("button", { name: "Session menu" })).toBeInTheDocument();

    // Ending again has no session to end: the failure becomes a toast, not a rejection.
    expect(await endSession()).toBe(false);
  });
});
