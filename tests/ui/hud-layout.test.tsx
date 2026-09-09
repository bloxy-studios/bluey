import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { HudPanel } from "@/features/hud/HudPanel";
import { eventBus } from "@/lib/tauri/event-bus";
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

describe("HUD layout boundaries", () => {
  beforeEach(async () => {
    await setupMockApp();
    setEngine(new FakeEngine());
  });

  it("keeps the compact surface inside the measured shadow frame with one opacity", () => {
    const settings = useSettingsStore.getState().settings!;
    useSettingsStore.setState({
      settings: { ...settings, appearance: { ...settings.appearance, opacity: 0.5 } },
    });
    renderHud();
    const surface = screen.getByRole("dialog", { name: "Bluey" });
    const frame = surface.parentElement!;
    expect(frame).toHaveAttribute("data-hud-frame");
    expect(frame).toHaveStyle({ width: "754px", paddingTop: "24px", paddingBottom: "40px" });
    expect(frame).toHaveClass("max-w-full");
    expect(frame).not.toHaveClass("h-full");
    expect(surface).toHaveClass("w-full", "min-w-0");
    expect(surface).toHaveStyle({ opacity: "0.5" });
    expect(frame.style.opacity).toBe("");
    expect(screen.getByRole("textbox", { name: "Ask Bluey" }).parentElement!.parentElement).toHaveClass(
      "shrink-0",
    );
  });

  it("keeps header and toolbar outside the shrinkable reading viewport and routes scroll shortcuts to it", () => {
    const generation = useChatStore.getState().begin("A long answer");
    useChatStore.getState().complete(generation, makeResponse({ content: "Paragraph\n\n".repeat(40) }));
    renderHud();
    const reading = screen.getByRole("region", { name: "Response" });
    expect(reading).toHaveClass("min-h-0", "overflow-y-auto");
    expect(reading).not.toHaveClass("max-h-[460px]");
    expect(reading.parentElement).toHaveClass("min-h-0", "flex-col");
    expect(reading).toHaveAttribute("tabindex", "0");
    const header = screen.getByRole("textbox", { name: "Ask follow-up" });
    expect(header.parentElement!.parentElement).toHaveClass("shrink-0");
    const toolbar = screen.getByText("New Chat");
    expect(reading).not.toContainElement(header);
    expect(reading).not.toContainElement(toolbar);

    const scroll = vi.spyOn(reading, "scrollBy");
    eventBus.emit("panel.scroll", { direction: "down" });
    expect(scroll).toHaveBeenLastCalledWith({ top: 160, behavior: "smooth" });
    eventBus.emit("panel.scroll", { direction: "up" });
    expect(scroll).toHaveBeenLastCalledWith({ top: -160, behavior: "smooth" });
    scroll.mockRestore();
  });
});
