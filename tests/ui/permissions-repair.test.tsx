import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import PermissionsTab from "@/features/settings/tabs/PermissionsTab";
import { bluey } from "@/lib/tauri/api";
import type { PermissionState } from "@/lib/types";
import { usePermissionsStore } from "@/stores/permissionsStore";
import { setupMockApp } from "./helpers";

const afterUpdate: PermissionState = {
  microphone: "granted",
  screenRecording: "denied",
  accessibility: "not_determined",
  notifications: "granted",
  speechRecognition: "granted",
  checkedAt: new Date().toISOString(),
  lostAfterUpdate: ["screenRecording", "accessibility"],
};

describe("Permissions repair card after an update (MAC-001)", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("stays hidden when no grant was lost", () => {
    render(<PermissionsTab />);
    expect(screen.queryByRole("region", { name: /after the update/ })).not.toBeInTheDocument();
  });

  it("names the lost grants, explains the reset and opens each pane", async () => {
    const user = userEvent.setup();
    const open = vi.spyOn(bluey.permissions, "openSettings");
    act(() => usePermissionsStore.getState().applyRemote(afterUpdate));
    render(<PermissionsTab />);

    const card = screen.getByRole("region", { name: "macOS turned off some permissions after the update" });
    expect(card.textContent).toContain("not signed with an Apple Developer ID");
    expect(card.textContent).toContain("Keychain");
    expect(card.textContent).not.toContain("Microphone");

    await user.click(screen.getByRole("button", { name: "Open System Settings for Screen Recording" }));
    await user.click(screen.getByRole("button", { name: "Open System Settings for Accessibility" }));
    expect(open).toHaveBeenNthCalledWith(1, { kind: "screenRecording" });
    expect(open).toHaveBeenNthCalledWith(2, { kind: "accessibility" });

    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("region", { name: /after the update/ })).not.toBeInTheDocument();
  });

  it("drops a permission from the card once it is granted again", async () => {
    const mock = await setupMockApp();
    act(() => mock.simulateLostAfterUpdate(["screenRecording", "accessibility"]));
    render(<PermissionsTab />);
    expect(screen.getByRole("button", { name: "Open System Settings for Screen Recording" })).toBeInTheDocument();

    await act(() => usePermissionsStore.getState().request("screenRecording"));
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Open System Settings for Screen Recording" })).toBeNull(),
    );
    expect(screen.getByRole("button", { name: "Open System Settings for Accessibility" })).toBeInTheDocument();
  });
});
