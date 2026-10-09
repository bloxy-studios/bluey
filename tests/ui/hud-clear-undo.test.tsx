import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { HudPanel } from "@/features/hud/HudPanel";
import { useChatStore } from "@/stores/chatStore";
import { setEngine } from "@/stores/engine";
import { useHudUiStore } from "@/stores/hudUiStore";
import { FakeEngine, makeResponse, setupMockApp } from "./helpers";

function renderHud() {
  return render(
    <TooltipProvider>
      <HudPanel />
    </TooltipProvider>,
  );
}

function showAnswer(content = "Because the bundle is lazy-loaded."): void {
  act(() => useChatStore.getState().showResponse(makeResponse({ content }), { promptLabel: "Why?" }));
}

describe("clearing the HUD chat (UX-012)", () => {
  beforeEach(async () => {
    await setupMockApp();
    setEngine(new FakeEngine());
  });

  it("Esc clears a half-typed follow-up first; only the next Esc clears the thread", async () => {
    const user = userEvent.setup();
    renderHud();
    showAnswer();
    const input = screen.getByRole("textbox", { name: "Ask follow-up" });
    await user.type(input, "and then");

    await user.keyboard("{Escape}");
    expect(input).toHaveValue("");
    expect(useChatStore.getState().turns).toHaveLength(1);

    await user.keyboard("{Escape}");
    expect(useChatStore.getState().turns).toHaveLength(0);
  });

  it("offers Chat cleared · Undo, which brings the thread back", async () => {
    const user = userEvent.setup();
    renderHud();
    showAnswer();
    await user.keyboard("{Escape}");

    expect(await screen.findByText("Chat cleared")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Undo" }));

    expect(useChatStore.getState().turns).toHaveLength(1);
    expect(await screen.findByText("Because the bundle is lazy-loaded.")).toBeInTheDocument();
    expect(screen.queryByText("Chat cleared")).not.toBeInTheDocument();
  });

  it("keeps the draft when the thread is cleared from the toolbar", async () => {
    const user = userEvent.setup();
    renderHud();
    showAnswer();
    act(() => useHudUiStore.getState().setDraft("keep me"));
    await user.click(screen.getByRole("button", { name: /new chat/i }));
    expect(useChatStore.getState().turns).toHaveLength(0);
    expect(screen.getByRole("textbox", { name: "Ask Bluey" })).toHaveValue("keep me");
  });
});
