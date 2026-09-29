import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { HudPanel } from "@/features/hud/HudPanel";
import { useChatStore } from "@/stores/chatStore";
import { setEngine } from "@/stores/engine";
import { FakeEngine, makeResponse, setupMockApp } from "./helpers";

function renderHud() {
  return render(
    <TooltipProvider>
      <HudPanel />
    </TooltipProvider>,
  );
}

/** What the proactive loop does when it hears a question: open a suggestion turn. */
function openLiveSuggestion(): void {
  act(() => {
    const generation = useChatStore
      .getState()
      .begin(undefined, "Suggestion", { phase: "thinking", suggestion: { question: "Why us?" } });
    useChatStore.getState().complete(generation, makeResponse({ content: "Because…" }));
  });
}

describe("HUD composer across layouts (LIVE-008)", () => {
  let engine: FakeEngine;

  beforeEach(async () => {
    await setupMockApp();
    engine = new FakeEngine();
    setEngine(engine);
  });

  it("keeps a half-typed question, its input and its focus when a live suggestion opens", async () => {
    const user = userEvent.setup();
    renderHud();
    const input = screen.getByRole("textbox", { name: "Ask Bluey" });
    await user.type(input, "what is");

    openLiveSuggestion();

    const composer = screen.getByRole("textbox", { name: "Ask follow-up" });
    expect(composer).toBe(input);
    expect(composer).toHaveValue("what is");
    expect(composer).toHaveFocus();
  });

  it("does not pull focus into the composer when a turn appears", () => {
    renderHud();
    // (Not a tooltip trigger: a focused Radix tooltip spins jsdom.)
    const submit = screen.getByRole("button", { name: "Submit" });
    act(() => submit.focus());

    openLiveSuggestion();
    expect(submit).toHaveFocus();
    expect(screen.getByRole("textbox", { name: "Ask follow-up" })).not.toHaveFocus();
  });

  it("asks a typed question after a suggestion as a fresh ask with the screen", async () => {
    const user = userEvent.setup();
    renderHud();
    openLiveSuggestion();

    await user.type(screen.getByRole("textbox", { name: "Ask follow-up" }), "and the salary?{Enter}");
    expect(engine.asks.at(-1)).toMatchObject({ trigger: "typed", captureScreen: true });

    await user.type(screen.getByRole("textbox", { name: "Ask follow-up" }), "thanks{Enter}");
    expect(engine.asks.at(-1)).toMatchObject({ trigger: "follow_up", captureScreen: false });
  });
});
