import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import KeybindsTab from "@/features/settings/tabs/KeybindsTab";
import { useSettingsStore } from "@/stores/settingsStore";
import { setupMockApp } from "./helpers";

describe("KeybindsTab", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("lists shortcut groups with keycaps", () => {
    render(<KeybindsTab />);
    expect(screen.getByText("Keyboard shortcuts")).toBeInTheDocument();
    expect(screen.getByText("General")).toBeInTheDocument();
    expect(screen.getByText("Window")).toBeInTheDocument();
    expect(screen.getByText("Scroll")).toBeInTheDocument();
    expect(screen.getByText("Toggle visibility of Bluey")).toBeInTheDocument();
  });

  it("records a new accelerator and saves it", async () => {
    const user = userEvent.setup();
    render(<KeybindsTab />);

    await user.click(screen.getByLabelText("Edit shortcut: Start a new chat"));
    expect(screen.getByText(/Press shortcut/)).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "j", metaKey: true, shiftKey: true });

    await waitFor(() => {
      const binding = useSettingsStore.getState().settings?.shortcuts.find((s) => s.id === "new_chat");
      expect(binding?.accelerator).toBe("CmdOrCtrl+Shift+J");
    });
    expect(screen.queryByText(/Press shortcut/)).not.toBeInTheDocument();
  });

  it("shows an inline warning on conflict and keeps the old binding", async () => {
    const user = userEvent.setup();
    render(<KeybindsTab />);

    // Record ⌘R (already used by "Start a new chat") on "Open Bluey settings".
    await user.click(screen.getByLabelText("Edit shortcut: Open Bluey settings"));
    fireEvent.keyDown(window, { key: "r", metaKey: true });

    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
    expect(screen.getByRole("alert").textContent).toContain("Start a new chat");
    const binding = useSettingsStore.getState().settings?.shortcuts.find((s) => s.id === "open_settings");
    expect(binding?.accelerator).toBe("CmdOrCtrl+Comma");
  });

  it("switches a shortcut off and on without touching its accelerator", async () => {
    const user = userEvent.setup();
    render(<KeybindsTab />);

    const toggle = screen.getByRole("switch", { name: "Enable shortcut: Start a new chat" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    await user.click(toggle);
    await waitFor(() => {
      const binding = useSettingsStore.getState().settings?.shortcuts.find((s) => s.id === "new_chat");
      expect(binding?.enabled).toBe(false);
      expect(binding?.accelerator).toBe("CmdOrCtrl+R");
    });

    await user.click(screen.getByRole("switch", { name: "Enable shortcut: Start a new chat" }));
    await waitFor(() => {
      expect(useSettingsStore.getState().settings?.shortcuts.find((s) => s.id === "new_chat")?.enabled).toBe(
        true,
      );
    });
  });

  it("escape cancels recording without changes", async () => {
    const user = userEvent.setup();
    render(<KeybindsTab />);
    await user.click(screen.getByLabelText("Edit shortcut: Start a new chat"));
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByText(/Press shortcut/)).not.toBeInTheDocument());
    const binding = useSettingsStore.getState().settings?.shortcuts.find((s) => s.id === "new_chat");
    expect(binding?.accelerator).toBe("CmdOrCtrl+R");
  });
});
