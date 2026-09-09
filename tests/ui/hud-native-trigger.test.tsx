import { act, createEvent, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { IconButton } from "@/components/ui/IconButton";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { useToastStore } from "@/components/ui/toast-store";
import { HudMenu } from "@/features/hud/HudMenu";
import { hasActiveHudOverlay } from "@/features/hud/hud-keyboard";
import { ModeMenu } from "@/features/hud/ModeMenu";
import { SessionMenu } from "@/features/hud/SessionMenu";
import { bluey } from "@/lib/tauri/api";
import type { HudMenuRequest } from "@/lib/tauri/hud-menu-types";
import { useAppStore } from "@/stores/appStore";
import { useSessionStore } from "@/stores/sessionStore";
import { setupInterceptedApp, type InterceptingTransport } from "./helpers";

const focus = vi.hoisted(() => ({ isFocused: vi.fn(), setFocus: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ label: "main", ...focus }) }));
let transport: InterceptingTransport;
beforeEach(async () => {
  ({ transport } = await setupInterceptedApp());
  // Even with mock domain data, the *runtime* determines whether the HUD uses native UI.
  Object.defineProperty(window, "__TAURI_INTERNALS__", { configurable: true, value: {} });
  focus.isFocused.mockResolvedValue(true);
  focus.setFocus.mockClear();
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  for (const toast of useToastStore.getState().toasts) useToastStore.getState().dismiss(toast.id);
});
afterEach(() => {
  Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
  vi.restoreAllMocks();
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => {
    resolve = yes;
  });
  return { promise, resolve };
}
function renderMenus() {
  return render(
    <TooltipProvider>
      <ModeMenu>
        <IconButton aria-label="Mode trigger" />
      </ModeMenu>
      <SessionMenu>
        <IconButton aria-label="Session trigger" />
      </SessionMenu>
    </TooltipProvider>,
  );
}
function selectTitle(title: string) {
  transport.intercept("hud_menu_popup", async ({ request }) => {
    const item = request.items.find((item) => item.kind === "item" && item.label === title);
    if (!item) throw new Error(`Missing display item ${title}`);
    return item.id;
  });
}
async function settle(button: HTMLElement) {
  await waitFor(() => expect(button).toHaveAttribute("aria-expanded", "false"));
}

describe("native HudMenu triggers and existing actions", () => {
  it("forwards the actual button ref, accessibility and child event handlers without wrapper spans", async () => {
    const ref = createRef<HTMLButtonElement>();
    const click = vi.fn();
    const tracking = deferred<string | null>();
    transport.intercept("hud_menu_popup", async () => tracking.promise);
    render(
      <TooltipProvider>
        <HudMenu
          entries={[{ kind: "item", id: "one", label: "One", action: 1 }]}
          onSelect={() => {}}
          tooltip="Menu"
        >
          <IconButton aria-label="Native" ref={ref} onClick={click} />
        </HudMenu>
      </TooltipProvider>,
    );
    const button = screen.getByRole("button", { name: "Native" });
    expect(ref.current).toBe(button);
    expect(button.tagName).toBe("BUTTON");
    expect(button).toHaveAttribute("type", "button");
    expect(button).toHaveAttribute("aria-haspopup", "menu");
    expect(button).toHaveAttribute("aria-expanded", "false");
    expect(button.parentElement?.tagName).not.toBe("SPAN");
    fireEvent.click(button);
    expect(click).toHaveBeenCalledOnce();
    expect(button).toHaveAttribute("aria-expanded", "true");
    expect(hasActiveHudOverlay()).toBe(true);
    expect(screen.queryByRole("menu")).not.toBeInTheDocument(); // no clipped DOM menu
    await act(async () => tracking.resolve(null));
    await settle(button);
    expect(hasActiveHudOverlay()).toBe(false);
  });

  it.each(["Enter", " ", "ArrowDown"])(
    "opens from %s and suppresses repeats, composing keys and child-prevented events",
    async (key) => {
      const popup = vi.fn(async () => null);
      transport.intercept("hud_menu_popup", popup);
      renderMenus();
      for (const name of ["Mode trigger", "Session trigger"]) {
        const button = screen.getByRole("button", { name });
        for (const flags of [{ repeat: true }, { isComposing: true }, { keyCode: 229 }]) {
          expect(fireEvent.keyDown(button, { key, ...flags })).toBe(false);
        }
        const prevented = createEvent.keyDown(button, { key });
        prevented.preventDefault();
        fireEvent(button, prevented);
        expect(button).toHaveAttribute("aria-expanded", "false");
        const before = popup.mock.calls.length;
        expect(fireEvent.keyDown(button, { key })).toBe(false);
        expect(popup).toHaveBeenCalledTimes(before + 1);
        await settle(button);
      }
      expect(popup).toHaveBeenCalledTimes(2);
    },
  );

  it("shares one guard across Mode and Session while the native command is pending", async () => {
    const tracking = deferred<string | null>();
    const popup = vi.fn(async () => tracking.promise);
    transport.intercept("hud_menu_popup", popup);
    renderMenus();
    const mode = screen.getByRole("button", { name: "Mode trigger" });
    const session = screen.getByRole("button", { name: "Session trigger" });
    fireEvent.click(mode);
    fireEvent.click(session);
    fireEvent.click(mode);
    expect(popup).toHaveBeenCalledOnce();
    expect(mode).toHaveAttribute("aria-expanded", "true");
    expect(session).toHaveAttribute("aria-expanded", "false");
    await act(async () => tracking.resolve(null));
    await settle(mode);
    fireEvent.click(session);
    await settle(session);
    expect(popup).toHaveBeenCalledTimes(2);
  });

  it("selects a mode using the unchanged domain action and active check, then Manage opens the same route", async () => {
    let shown: HudMenuRequest | undefined;
    transport.intercept("hud_menu_popup", async ({ request }) => {
      shown = request;
      const item = request.items.find((item) => item.kind === "item" && item.label === "Interview");
      return item?.id ?? null;
    });
    renderMenus();
    const mode = screen.getByRole("button", { name: "Mode trigger" });
    fireEvent.click(mode);
    await waitFor(() => expect(useAppStore.getState().status?.modeId).toBe("interview"));
    expect(shown?.items.find((item) => item.kind === "item" && item.label === "General")).toMatchObject({
      checked: true,
    });
    await settle(mode);
    const opened = vi.fn(async () => {});
    transport.intercept("window_open", opened);
    selectTitle("Manage");
    mode.focus();
    fireEvent.keyDown(mode, { key: "ArrowDown" });
    await waitFor(() =>
      expect(opened).toHaveBeenCalledWith({ label: "settings", route: "modes" }, expect.any(Function)),
    );
    await settle(mode);
    expect(focus.setFocus).not.toHaveBeenCalled();
  });

  it("keeps session start/pause/resume/end/history and guarded audio coordination unchanged", async () => {
    renderMenus();
    const button = screen.getByRole("button", { name: "Session trigger" });
    selectTitle("Start session");
    fireEvent.click(button);
    await waitFor(() => expect(useSessionStore.getState().active?.status).toBe("active"));
    await settle(button);
    await act(async () => {
      await bluey.audio.start();
    });
    const calls = vi.spyOn(transport, "invoke");
    for (const [title, status] of [
      ["Pause session", "paused"],
      ["Resume session", "active"],
    ] as const) {
      selectTitle(title);
      fireEvent.click(button);
      await waitFor(() => expect(useSessionStore.getState().active?.status).toBe(status));
      await settle(button);
    }
    expect(calls).toHaveBeenCalledWith("sessions_pause", undefined);
    expect(calls).toHaveBeenCalledWith("audio_pause", undefined);
    expect(calls).toHaveBeenCalledWith("sessions_resume", undefined);
    expect(calls).toHaveBeenCalledWith("audio_resume", undefined);
    selectTitle("End session");
    fireEvent.click(button);
    await waitFor(() => expect(useSessionStore.getState().active).toBeNull());
    await settle(button);
    const opened = vi.fn(async () => {});
    transport.intercept("window_open", opened);
    selectTitle("Open History");
    fireEvent.click(button);
    await waitFor(() =>
      expect(opened).toHaveBeenCalledWith({ label: "settings", route: "sessions" }, expect.any(Function)),
    );
    await settle(button);
  });

  it("surfaces native creation failures, releases expanded state and permits a retry", async () => {
    const popup = vi.fn().mockRejectedValueOnce(new Error("native menu unavailable")).mockResolvedValue(null);
    transport.intercept("hud_menu_popup", popup);
    renderMenus();
    const button = screen.getByRole("button", { name: "Mode trigger" });
    fireEvent.click(button);
    await waitFor(() =>
      expect(useToastStore.getState().toasts.some((toast) => toast.variant === "error")).toBe(true),
    );
    await settle(button);
    fireEvent.click(button);
    await settle(button);
    expect(popup).toHaveBeenCalledTimes(2);
  });

  it("does not select/refocus an unmounted trigger or release its guard early", async () => {
    const tracking = deferred<string | null>();
    const popup = vi.fn(async () => tracking.promise);
    transport.intercept("hud_menu_popup", popup);
    const select = vi.fn();
    const view = render(
      <HudMenu entries={[{ kind: "item", id: "one", label: "One", action: 1 }]} onSelect={select}>
        <IconButton aria-label="Old" />
      </HudMenu>,
    );
    const old = screen.getByRole("button", { name: "Old" });
    old.focus();
    const restore = vi.spyOn(old, "focus");
    fireEvent.click(old);
    view.unmount();
    renderMenus();
    const next = screen.getByRole("button", { name: "Mode trigger" });
    fireEvent.click(next);
    expect(popup).toHaveBeenCalledOnce();
    await act(async () => tracking.resolve("hud-0"));
    expect(select).not.toHaveBeenCalled();
    expect(restore).not.toHaveBeenCalled();
    transport.intercept("hud_menu_popup", async () => null);
    fireEvent.click(next);
    await settle(next);
  });

  it("keeps the browser MockTransport on the existing Radix fallback", () => {
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    renderMenus();
    for (const name of ["Mode trigger", "Session trigger"]) {
      const button = screen.getByRole("button", { name });
      expect(button).toHaveAttribute("aria-haspopup", "menu");
      expect(button).toHaveAttribute("data-state", "closed");
      expect(button).not.toHaveAttribute("data-native-hud-menu");
    }
  });
});
