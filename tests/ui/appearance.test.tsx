import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import AppearanceTab from "@/features/settings/tabs/AppearanceTab";
import { bluey } from "@/lib/tauri/api";
import { useSettingsStore } from "@/stores/settingsStore";
import { setupMockApp } from "./helpers";

describe("AppearanceTab", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("persists theme, blur and density changes", async () => {
    const user = userEvent.setup();
    render(<AppearanceTab />);

    await user.selectOptions(screen.getByLabelText("Theme"), "light");
    await waitFor(() => expect(useSettingsStore.getState().settings?.appearance.theme).toBe("light"));

    await user.click(screen.getByLabelText("Background blur"));
    await waitFor(() => expect(useSettingsStore.getState().settings?.appearance.blur).toBe(false));

    await user.click(screen.getByRole("tab", { name: "Compact" }));
    await waitFor(() => expect(useSettingsStore.getState().settings?.appearance.density).toBe("compact"));

    // Round-trip through the transport, other sections untouched.
    const persisted = await bluey.settings.get();
    expect(persisted.appearance).toMatchObject({
      theme: "light",
      blur: false,
      density: "compact",
      width: 690,
    });
    expect(persisted.general.blueyName).toBe("Bluey");
  });

  it("shows the current opacity and width and exposes the sliders", () => {
    render(<AppearanceTab />);
    expect(screen.getByText(/100% — the panel keeps its blur/)).toBeInTheDocument();
    expect(screen.getByText("690 px")).toBeInTheDocument();
    expect(screen.getByRole("slider", { name: "Panel opacity" })).toHaveAttribute("aria-valuenow", "1");
    expect(screen.getByRole("slider", { name: "Panel width" })).toHaveAttribute("aria-valuenow", "690");
  });

  it("changes reduced motion and position", async () => {
    const user = userEvent.setup();
    render(<AppearanceTab />);
    await user.selectOptions(screen.getByLabelText("Reduced motion"), "on");
    await user.selectOptions(screen.getByLabelText("Panel position"), "top");
    await waitFor(() => {
      const appearance = useSettingsStore.getState().settings?.appearance;
      expect(appearance?.reducedMotion).toBe("on");
      expect(appearance?.position).toBe("top");
    });
  });
});
