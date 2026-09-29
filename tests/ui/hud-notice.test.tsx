import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { Toasts } from "@/components/ui/Toast";
import { showErrorToast, showToast, useToastStore } from "@/components/ui/toast-store";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { HudPanel } from "@/features/hud/HudPanel";
import type { BlueyError } from "@/lib/types";
import { setEngine } from "@/stores/engine";
import { FakeEngine, setupMockApp } from "./helpers";

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
