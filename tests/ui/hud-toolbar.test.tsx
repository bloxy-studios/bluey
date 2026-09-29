import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { HudPanel } from "@/features/hud/HudPanel";
import type { CommandArgs } from "@/lib/tauri/commands";
import type { MockTransport } from "@/lib/tauri/mock";
import { bluey } from "@/lib/tauri/api";
import { setEngine } from "@/stores/engine";
import { useSettingsStore } from "@/stores/settingsStore";
import { FakeEngine, setupInterceptedApp, type InterceptingTransport } from "./helpers";

function renderHud() {
  return render(
    <TooltipProvider>
      <HudPanel />
    </TooltipProvider>,
  );
}

describe("HUD content-protection eye (UX-004)", () => {
  let mock: MockTransport;
  let transport: InterceptingTransport;
  let updates: Array<CommandArgs<"settings_update">>;

  beforeEach(async () => {
    ({ mock, transport } = await setupInterceptedApp());
    setEngine(new FakeEngine());
    updates = [];
    transport.intercept("settings_update", (args, next) => {
      updates.push(args);
      return next();
    });
  });

  it("shows the saved display mode, and follows it when it changes elsewhere", async () => {
    renderHud();
    expect(await screen.findByRole("button", { name: "Content protection off" })).toBeEnabled();

    // Settings → Privacy or the tray saved it: settings.changed carries the new mode.
    const settings = useSettingsStore.getState().settings!;
    act(() => mock.emit("settings.changed", { ...settings, privacy: { ...settings.privacy, displayMode: "privacy" } }));
    expect(await screen.findByRole("button", { name: "Content protection on" })).toBeInTheDocument();
  });

  it("saves the toggle as the display mode, so it survives a relaunch", async () => {
    const user = userEvent.setup();
    renderHud();
    await user.click(await screen.findByRole("button", { name: "Content protection off" }));

    await waitFor(() => expect(updates).toEqual([{ patch: { privacy: { displayMode: "privacy" } } }]));
    expect(await screen.findByRole("button", { name: "Content protection on" })).toBeInTheDocument();
    // The backend applied it (settings side effect), so the capture state agrees.
    expect((await bluey.capture.getProtection()).enabled).toBe(true);
  });
});
