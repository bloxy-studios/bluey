import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { useToastStore } from "@/components/ui/toast-store";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { HudPanel } from "@/features/hud/HudPanel";
import type { CommandArgs } from "@/lib/tauri/commands";
import type { MockTransport } from "@/lib/tauri/mock";
import { bluey } from "@/lib/tauri/api";
import { setEngine } from "@/stores/engine";
import { useAppStore } from "@/stores/appStore";
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

describe("HUD state pill recovery (UX-036)", () => {
  it("still clears the error and shows why when the recovery itself fails", async () => {
    const { mock, transport } = await setupInterceptedApp();
    setEngine(new FakeEngine());
    useToastStore.setState({ toasts: [] });
    let recovered = 0;
    transport.intercept("window_open", async () => {
      throw { kind: "internal", code: "internal.window", message: "The Settings window could not open.", recoverable: false };
    });
    transport.intercept("app_recover", (_args, next) => {
      recovered += 1;
      return next();
    });
    renderHud();

    const { status } = useAppStore.getState();
    act(() =>
      mock.emit("app.state", {
        ...status!,
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
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Open Settings" }));

    await waitFor(() => expect(recovered).toBe(1));
    await waitFor(() =>
      expect(useToastStore.getState().toasts).toEqual([expect.objectContaining({ variant: "error" })]),
    );
  });
});
