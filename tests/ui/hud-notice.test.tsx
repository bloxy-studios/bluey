import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { Toasts } from "@/components/ui/Toast";
import { showErrorToast, showToast, useToastStore } from "@/components/ui/toast-store";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { HudPanel } from "@/features/hud/HudPanel";
import type { MockTransport } from "@/lib/tauri/mock";
import type { BlueyError, ContextSnapshot } from "@/lib/types";
import { setEngine } from "@/stores/engine";
import { FakeEngine, setupInterceptedApp, setupMockApp } from "./helpers";

const micDenied: BlueyError = {
  kind: "permission",
  code: "permission.microphone",
  message: "Microphone permission is denied.",
  recoverable: true,
  recovery: { type: "open_system_settings", pane: "microphone" },
};

function renderHudWindow() {
  return render(
    <TooltipProvider>
      <HudPanel />
      <Toasts limit={1} />
    </TooltipProvider>,
  );
}

describe("HUD notice row (UX-013)", () => {
  beforeEach(async () => {
    await setupMockApp();
    setEngine(new FakeEngine());
    useToastStore.setState({ toasts: [] });
  });

  it("shows errors inside the HUD surface, above the toolbar, never as an overlay", async () => {
    renderHudWindow();
    act(() => showErrorToast(micDenied));

    const alert = await screen.findByRole("alert");
    // One alert, inside the measured surface (the dialog), before the toolbar.
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    const surface = screen.getByRole("dialog", { name: "Bluey" });
    expect(surface).toContainElement(alert);
    const audio = within(surface).getByRole("button", { name: "Start audio session" });
    expect(alert.compareDocumentPosition(audio) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("runs the recovery and dismisses the notice", async () => {
    const user = userEvent.setup();
    renderHudWindow();
    act(() => showErrorToast(micDenied));
    await user.click(await screen.findByRole("button", { name: "Open System Settings" }));
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
  });

  it("keeps a confirmation as a single overlay pill", async () => {
    renderHudWindow();
    act(() => {
      showToast("Copied");
      showToast("Saved");
    });
    const statuses = await screen.findAllByText(/Copied|Saved/);
    expect(statuses.map((node) => node.textContent)).toEqual(["Saved"]);
  });

  it("leaves other windows' overlay toasts alone", async () => {
    render(<Toasts />);
    act(() => showErrorToast(micDenied));
    expect(await screen.findByRole("alert")).toHaveTextContent("Open System Settings");
  });
});

describe("screen context notice (UX-002)", () => {
  let mock: MockTransport;
  let opened: string[];

  const deniedSnapshot: ContextSnapshot = {
    timestamp: new Date().toISOString(),
    warnings: [
      {
        kind: "screen_unavailable",
        code: "permission.screen_recording",
        message: "Screen Recording permission is not granted.",
        recovery: { type: "open_system_settings", pane: "screenRecording" },
      },
    ],
  };

  beforeEach(async () => {
    const setup = await setupInterceptedApp();
    mock = setup.mock;
    opened = [];
    setup.transport.intercept("permissions_open_settings", async (args) => {
      opened.push(args.kind);
    });
    setEngine(new FakeEngine());
    useToastStore.setState({ toasts: [] });
  });

  it("says the screen was not included and opens Screen Recording settings", async () => {
    const user = userEvent.setup();
    renderHudWindow();
    act(() => mock.emit("context.updated", { snapshot: deniedSnapshot, reason: "manual" }));

    const notice = (await screen.findByText("Screen not included")).closest<HTMLElement>('[role="status"]');
    expect(notice).not.toBeNull();
    await user.click(within(notice!).getByRole("button", { name: "Open System Settings" }));
    await waitFor(() => expect(opened).toEqual(["screenRecording"]));
    expect(screen.queryByText("Screen not included")).not.toBeInTheDocument();
  });

  it("clears when a later snapshot has the screen, or when Screen is turned off", async () => {
    const user = userEvent.setup();
    renderHudWindow();
    act(() => mock.emit("context.updated", { snapshot: deniedSnapshot, reason: "manual" }));
    expect(await screen.findByText("Screen not included")).toBeInTheDocument();
    act(() => mock.emit("context.updated", { snapshot: { timestamp: deniedSnapshot.timestamp }, reason: "manual" }));
    await waitFor(() => expect(screen.queryByText("Screen not included")).not.toBeInTheDocument());

    act(() => mock.emit("context.updated", { snapshot: deniedSnapshot, reason: "manual" }));
    expect(await screen.findByText("Screen not included")).toBeInTheDocument();
    const toggle = screen.getByRole("button", { name: "Screen context" });
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByText("Screen not included")).not.toBeInTheDocument();
  });
});
