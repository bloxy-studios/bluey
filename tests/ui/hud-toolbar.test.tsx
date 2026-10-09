import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useToastStore } from "@/components/ui/toast-store";
import type * as TooltipModule from "@/components/ui/Tooltip";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { HudPanel } from "@/features/hud/HudPanel";
import type { CommandArgs } from "@/lib/tauri/commands";
import type { MockTransport } from "@/lib/tauri/mock";
import { bluey } from "@/lib/tauri/api";
import { setEngine } from "@/stores/engine";
import { useAppStore } from "@/stores/appStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { FakeEngine, setupInterceptedApp, type InterceptingTransport } from "./helpers";

// Radix tooltips do not open in jsdom: expose each label on a wrapper instead.
vi.mock("@/components/ui/Tooltip", async (importActual) => ({
  ...(await importActual<typeof TooltipModule>()),
  Tooltip: ({ label, children }: { label: string; children: React.ReactNode }) => (
    <span data-tooltip={label}>{children}</span>
  ),
}));

const tooltipOf = (element: HTMLElement) => element.closest("[data-tooltip]")?.getAttribute("data-tooltip");

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
    // A full-protection platform (macOS 14) that has not applied the new mode yet.
    transport.intercept("capture_get_protection", async (_args, next) => ({ ...(await next()), partial: false }));
    renderHud();
    const eye = await screen.findByRole("button", { name: "Content protection" });
    expect(eye).toBeEnabled();
    expect(eye).toHaveAttribute("aria-pressed", "false");

    // Settings → Privacy or the tray saved it: settings.changed carries the new mode.
    const settings = useSettingsStore.getState().settings!;
    act(() => mock.emit("settings.changed", { ...settings, privacy: { ...settings.privacy, displayMode: "privacy" } }));
    await waitFor(() => expect(eye).toHaveAttribute("aria-pressed", "true"));
    // The tray publishes settings just before its side effect applies them: a status that
    // disagrees with the saved mode is unknown, never "Content-protected" (SEC-004).
    await waitFor(() => expect(tooltipOf(eye)).toBe("Privacy mode: hidden from legacy capture only"));
    // Once the side effect applied it, a re-read finds full protection.
    await act(() => bluey.capture.setProtection({ enabled: true }));
    await waitFor(() => expect(tooltipOf(eye)).toBe("Content-protected"));
  });

  it("saves the toggle as the display mode, so it survives a relaunch", async () => {
    const user = userEvent.setup();
    renderHud();
    const eye = await screen.findByRole("button", { name: "Content protection", pressed: false });
    await user.click(eye);

    await waitFor(() => expect(updates).toEqual([{ patch: { privacy: { displayMode: "privacy" } } }]));
    await waitFor(() => expect(eye).toHaveAttribute("aria-pressed", "true"));
    // The backend applied it (settings side effect), so the capture state agrees.
    expect((await bluey.capture.getProtection()).enabled).toBe(true);
  });

  // Rust reports `partial` on macOS 15+ (ScreenCaptureKit may still capture Bluey): the eye
  // must not call that "Content-protected", after a toggle from anywhere (SEC-004).
  it("says partial protection is legacy-capture only, and re-reads it when the mode changes", async () => {
    const user = userEvent.setup();
    let reads = 0;
    let partial = true;
    transport.intercept("capture_get_protection", async (_args, next) => {
      reads += 1;
      const status = await next();
      return { ...status, partial: status.enabled && partial };
    });
    renderHud();
    const eye = await screen.findByRole("button", { name: "Content protection" });
    await waitFor(() => expect(reads).toBe(1));

    await user.click(eye);
    await waitFor(() => expect(reads).toBe(2));
    await waitFor(() => expect(tooltipOf(eye)).toBe("Privacy mode: hidden from legacy capture only"));

    // macOS 14: Rust reports full protection.
    partial = false;
    await user.click(eye);
    await waitFor(() => expect(eye).toHaveAttribute("aria-pressed", "false"));
    expect(tooltipOf(eye)).toBe("Detectable");
    await user.click(eye);
    await waitFor(() => expect(reads).toBe(4));
    await waitFor(() => expect(tooltipOf(eye)).toBe("Content-protected"));
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
