import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { answerSummary } from "@/features/hud/announce";
import { HudPanel } from "@/features/hud/HudPanel";
import { useAppStore } from "@/stores/appStore";
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

const announcer = () => screen.getByTestId("hud-announcer");

describe("HUD screen-reader announcer (UX-025)", () => {
  beforeEach(async () => {
    await setupMockApp();
    setEngine(new FakeEngine());
  });

  it("says Bluey is thinking once, with the spinners hidden, then Answer ready — not every token", async () => {
    renderHud();
    let generation = 0;
    act(() => {
      generation = useChatStore.getState().begin("Why is the build slow?", undefined, { phase: "thinking" });
    });
    expect(announcer()).toHaveTextContent("Bluey is thinking");
    expect(announcer()).toHaveAttribute("aria-live", "polite");
    // The pill and the thread spinner no longer announce a second, meaningless "Loading".
    expect(screen.getAllByRole("status")).toEqual([announcer()]);

    act(() => useChatStore.getState().setPhase(generation, "streaming"));
    for (const content of ["Because", "Because the", "Because the cache is cold"]) {
      act(() => {
        useChatStore.getState().applyDraft(generation, makeResponse({ content }));
        useChatStore.getState().flushDraft();
      });
      expect(announcer()).toHaveTextContent(/^Bluey is thinking$/);
    }

    act(() =>
      useChatStore
        .getState()
        .complete(generation, makeResponse({ content: "Because the cache is cold. Warm it first." })),
    );
    expect(announcer()).toHaveTextContent(/^Answer ready: Because the cache is cold\.$/);
  });

  it("announces Listening when an audio session starts", () => {
    renderHud();
    act(() =>
      useAppStore.setState({
        status: { state: "listening", audioActive: true, modeId: "general", updatedAt: new Date().toISOString() },
      }),
    );
    expect(announcer()).toHaveTextContent("Bluey is listening");
  });
});

describe("answerSummary", () => {
  it("prefers the title, else the first sentence without markdown", () => {
    expect(answerSummary(makeResponse({ title: "Cold cache" }))).toBe("Cold cache");
    expect(answerSummary(makeResponse({ content: "## Why\n**Because** it is `cold`. More." }))).toBe(
      "Why Because it is cold.",
    );
  });

  it("caps long answers and skips code blocks", () => {
    const summary = answerSummary(makeResponse({ content: "```ts\nconst x = 1;\n```\n" + "word ".repeat(80) }));
    expect(summary.length).toBeLessThanOrEqual(140);
    expect(summary).not.toContain("const");
    expect(summary.endsWith("…")).toBe(true);
  });
});
