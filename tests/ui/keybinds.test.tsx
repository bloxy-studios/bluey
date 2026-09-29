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
      expect(binding?.accelerator).toBe("CmdOrCtrl+Shift+KeyJ");
    });
    expect(screen.queryByText(/Press shortcut/)).not.toBeInTheDocument();
  });

  it("shows an inline warning on conflict and keeps the old binding", async () => {
    const user = userEvent.setup();
    render(<KeybindsTab />);

    // Record ⌘⇧L (already used by "Start or stop listening") on "Open Bluey settings".
    await user.click(screen.getByLabelText("Edit shortcut: Open Bluey settings"));
    fireEvent.keyDown(window, { key: "l", metaKey: true, shiftKey: true });

    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
    expect(screen.getByRole("alert").textContent).toContain("Start or stop listening");
    const binding = useSettingsStore.getState().settings?.shortcuts.find((s) => s.id === "open_settings");
    expect(binding?.accelerator).toBe("CmdOrCtrl+Comma");
  });

  it("does not take editing chords by default and warns when one is recorded (UX-001)", async () => {
    const user = userEvent.setup();
    render(<KeybindsTab />);

    // New Chat / Settings are HUD-local, Move is ⌃⌥ arrows.
    expect(screen.getByRole("switch", { name: "Enable shortcut: Start a new chat" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    expect(screen.getByRole("switch", { name: "Enable shortcut: Open Bluey settings" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    const moveLeft = screen.getByLabelText("Edit shortcut: Move the window position left");
    expect(moveLeft.textContent).toContain("⌃⌥←");

    await user.click(moveLeft);
    fireEvent.keyDown(window, { key: "ArrowLeft", metaKey: true });
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("start of the line"));
    const binding = useSettingsStore.getState().settings?.shortcuts.find((s) => s.id === "move_left");
    expect(binding?.accelerator).toBe("Ctrl+Alt+ArrowLeft");
  });

  it("records ⌃ as Control, not as ⌘", async () => {
    const user = userEvent.setup();
    render(<KeybindsTab />);
    await user.click(screen.getByLabelText("Edit shortcut: Move the window position up"));
    fireEvent.keyDown(window, { key: "k", ctrlKey: true, altKey: true });
    await waitFor(() => {
      const binding = useSettingsStore.getState().settings?.shortcuts.find((s) => s.id === "move_up");
      expect(binding?.accelerator).toBe("Ctrl+Alt+KeyK");
    });
  });

  it("switches a shortcut on and off without touching its accelerator", async () => {
    const user = userEvent.setup();
    render(<KeybindsTab />);

    const toggle = screen.getByRole("switch", { name: "Enable shortcut: Start a new chat" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    await user.click(toggle);
    await waitFor(() => {
      const binding = useSettingsStore.getState().settings?.shortcuts.find((s) => s.id === "new_chat");
      expect(binding?.enabled).toBe(true);
      expect(binding?.accelerator).toBe("CmdOrCtrl+KeyR");
    });

    await user.click(screen.getByRole("switch", { name: "Enable shortcut: Start a new chat" }));
    await waitFor(() => {
      expect(useSettingsStore.getState().settings?.shortcuts.find((s) => s.id === "new_chat")?.enabled).toBe(
        false,
      );
    });
  });

  it("marks a shortcut macOS did not register instead of hiding it in the dev log (UX-039)", async () => {
    const settings = useSettingsStore.getState().settings!;
    useSettingsStore.setState({
      settings: {
        ...settings,
        shortcuts: settings.shortcuts.map((s) =>
          s.id === "toggle_panel" ? { ...s, registrationError: "registration failed: HotKey already registered" } : s,
        ),
      },
    });
    render(<KeybindsTab />);
    expect(screen.getByText("Not active")).toBeInTheDocument();
    expect(screen.getByText(/macOS did not register this shortcut/).textContent).toContain(
      "HotKey already registered",
    );
    expect(screen.getAllByText("Not active")).toHaveLength(1);
  });

  it("escape cancels recording without changes", async () => {
    const user = userEvent.setup();
    render(<KeybindsTab />);
    await user.click(screen.getByLabelText("Edit shortcut: Start a new chat"));
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByText(/Press shortcut/)).not.toBeInTheDocument());
    const binding = useSettingsStore.getState().settings?.shortcuts.find((s) => s.id === "new_chat");
    expect(binding?.accelerator).toBe("CmdOrCtrl+KeyR");
  });
});
