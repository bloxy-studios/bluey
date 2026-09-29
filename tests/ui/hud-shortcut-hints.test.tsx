import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { HudPanel } from "@/features/hud/HudPanel";
import type { ShortcutId } from "@/lib/types";
import { acceleratorToGlyphs } from "@/lib/utils/keyboard";
import { useChatStore } from "@/stores/chatStore";
import { setEngine } from "@/stores/engine";
import { useSettingsStore } from "@/stores/settingsStore";
import { FakeEngine, makeResponse, setupMockApp } from "./helpers";

function renderHud() {
  return render(
    <TooltipProvider>
      <HudPanel />
    </TooltipProvider>,
  );
}

function rebind(id: ShortcutId, change: { accelerator?: string; enabled?: boolean }): void {
  const settings = useSettingsStore.getState().settings!;
  act(() =>
    useSettingsStore.setState({
      settings: {
        ...settings,
        shortcuts: settings.shortcuts.map((binding) => (binding.id === id ? { ...binding, ...change } : binding)),
      },
    }),
  );
}

function showAnswer(): void {
  act(() => useChatStore.getState().showResponse(makeResponse(), { promptLabel: "Why?" }));
}

describe("HUD shortcut hints follow Settings → Keybinds (UX-026)", () => {
  beforeEach(async () => {
    await setupMockApp();
    setEngine(new FakeEngine());
  });

  it("shows the saved New Chat binding, and none when it is disabled", () => {
    renderHud();
    showAnswer();
    rebind("new_chat", { accelerator: "CmdOrCtrl+Shift+KeyN" });
    const button = screen.getByRole("button", { name: "New Chat" });
    expect(within(button).getByLabelText("⌘ ⇧ N")).toBeInTheDocument();

    rebind("new_chat", { enabled: false });
    expect(within(button).queryByLabelText(/⌘/)).not.toBeInTheDocument();
  });

  it("the suggestion pill names the saved Generate binding and shows the prepared answer on click", async () => {
    const user = userEvent.setup();
    renderHud();
    rebind("generate_response", { accelerator: "CmdOrCtrl+Alt+KeyG" });
    const prepared = makeResponse({ id: "prep-1", content: "Say the cache is cold.", prepared: true });
    act(() => useChatStore.getState().setPrepared(prepared));

    const pill = screen.getByRole("button", { name: "Show Bluey's suggestion" });
    expect(pill).toHaveTextContent("Bluey has a suggestion · ⌘⌥G");
    await user.click(pill);

    expect(await screen.findByText("Say the cache is cold.")).toBeInTheDocument();
    expect(useChatStore.getState().prepared).toBeNull();
  });
});

describe("acceleratorToGlyphs", () => {
  it("renders the key codes the Rust defaults save as single keycaps", () => {
    expect(acceleratorToGlyphs("CmdOrCtrl+KeyR")).toEqual(["⌘", "R"]);
    expect(acceleratorToGlyphs("CmdOrCtrl+Digit1")).toEqual(["⌘", "1"]);
    expect(acceleratorToGlyphs("CmdOrCtrl+Shift+Enter")).toEqual(["⌘", "⇧", "↵"]);
  });
});
