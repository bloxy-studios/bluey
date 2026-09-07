import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import GeneralTab from "@/features/settings/tabs/GeneralTab";
import { ModeEditor } from "@/features/settings/ModeEditor";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { bluey } from "@/lib/tauri/api";
import { useAppStore } from "@/stores/appStore";
import { useModesStore } from "@/stores/modesStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { setupMockApp } from "./helpers";

describe("GeneralTab", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("toggling a switch persists through bluey.settings.update", async () => {
    const user = userEvent.setup();
    render(<GeneralTab />);

    expect(useSettingsStore.getState().settings?.general.launchAtLogin).toBe(false);
    await user.click(screen.getByLabelText("Launch Bluey at login"));

    await waitFor(() => {
      expect(useSettingsStore.getState().settings?.general.launchAtLogin).toBe(true);
    });
    // The backend agrees (round-trip through the transport).
    expect((await bluey.settings.get()).general.launchAtLogin).toBe(true);
  });
});

describe("ModeEditor", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("saves meeting-context edits (debounced) via modes.update", async () => {
    const user = userEvent.setup();
    const mode = useModesStore.getState().modes.find((m) => m.id === "coding-interview");
    expect(mode).toBeDefined();
    if (!mode) return;

    render(
      <TooltipProvider>
        <ModeEditor mode={mode} isActive={false} onDeleted={() => {}} />
      </TooltipProvider>,
    );

    const textarea = screen.getByLabelText("Meeting context");
    await user.clear(textarea);
    await user.type(textarea, "Focus on graph problems.");

    await waitFor(
      async () => {
        const updated = await bluey.modes.get({ id: "coding-interview" });
        expect(updated.systemInstructions).toBe("Focus on graph problems.");
      },
      { timeout: 2500 },
    );
  });

  it("Set Active activates the mode", async () => {
    const user = userEvent.setup();
    const mode = useModesStore.getState().modes.find((m) => m.id === "sales");
    if (!mode) throw new Error("sales mode missing");

    render(
      <TooltipProvider>
        <ModeEditor mode={mode} isActive={false} onDeleted={() => {}} />
      </TooltipProvider>,
    );

    await user.click(screen.getByRole("button", { name: "Set Active" }));
    await waitFor(() => expect(useAppStore.getState().status?.modeId).toBe("sales"));
  });
});
