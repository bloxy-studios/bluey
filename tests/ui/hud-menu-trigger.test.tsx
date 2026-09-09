import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { HudToolbar } from "@/features/hud/HudToolbar";
import { setupMockApp } from "./helpers";

beforeEach(async () => {
  await setupMockApp();
});

describe("HUD menu trigger composition", () => {
  it("puts menu semantics and focusability on the actual toolbar buttons, not wrapping spans", async () => {
    render(
      <TooltipProvider>
        <HudToolbar screenEnabled onToggleScreen={() => {}} hasChat={false} onNewChat={() => {}} />
      </TooltipProvider>,
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /^Content protection (on|off)$/ })).not.toBeDisabled(),
    );
    for (const name of ["Mode: General", "Session menu"]) {
      const trigger = screen.getByRole("button", { name });
      expect(trigger.tagName).toBe("BUTTON");
      expect(trigger).toHaveAttribute("type", "button");
      expect(trigger).toHaveAttribute("aria-haspopup", "menu");
      expect(trigger).toHaveAttribute("aria-expanded", "false");
      expect(trigger.parentElement?.tagName).not.toBe("SPAN");
      expect(trigger.tabIndex).toBe(0);
    }
  });

  it("prevents repeated new-chat activation without disabling fresh clicks", async () => {
    const newChat = vi.fn();
    render(
      <TooltipProvider>
        <HudToolbar screenEnabled onToggleScreen={() => {}} hasChat onNewChat={newChat} />
      </TooltipProvider>,
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /^Content protection (on|off)$/ })).not.toBeDisabled(),
    );
    const button = screen.getByRole("button", { name: /New Chat/ });
    expect(fireEvent.keyDown(button, { key: "Enter", repeat: true })).toBe(false);
    expect(fireEvent.keyDown(button, { key: " ", repeat: true })).toBe(false);
    expect(newChat).not.toHaveBeenCalled();
    fireEvent.click(button);
    expect(newChat).toHaveBeenCalledOnce();
  });

  it("does not open either menu from a repeated trigger activation", async () => {
    render(
      <TooltipProvider>
        <HudToolbar screenEnabled onToggleScreen={() => {}} hasChat={false} onNewChat={() => {}} />
      </TooltipProvider>,
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /^Content protection (on|off)$/ })).not.toBeDisabled(),
    );
    for (const name of ["Mode: General", "Session menu"]) {
      const trigger = screen.getByRole("button", { name });
      expect(fireEvent.keyDown(trigger, { key: "Enter", repeat: true })).toBe(false);
      expect(fireEvent.keyDown(trigger, { key: " ", repeat: true })).toBe(false);
      expect(trigger).toHaveAttribute("aria-expanded", "false");
    }
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });
});
