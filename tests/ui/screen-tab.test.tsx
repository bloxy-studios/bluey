import { render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import ScreenTab from "@/features/settings/tabs/ScreenTab";
import { useSettingsStore } from "@/stores/settingsStore";
import { setupMockApp } from "./helpers";

describe("ScreenTab capture target (FEATURE-006)", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("offers only targets Bluey can actually capture", async () => {
    render(<ScreenTab />);

    const select = await screen.findByLabelText("Capture target");
    const labels = within(select)
      .getAllByRole("option")
      .map((o) => o.textContent);
    expect(labels).toEqual(["Full display", "Active window"]);
  });
});

describe("ScreenTab observation (FEATURE-002, UX-031)", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("shows Smart observation as not yet available instead of offering it", async () => {
    render(<ScreenTab />);

    const select = await screen.findByLabelText("Observation mode");
    const smart = within(select).getByRole("option", { name: "Smart (not yet available)" });
    expect(smart).toBeDisabled();
    expect(within(select).getByRole("option", { name: "Manual" })).toBeEnabled();
    expect(screen.queryByLabelText("Observation interval")).toBeNull();
  });

  it("keeps a stored Smart value honest and offers the default interval", async () => {
    await useSettingsStore.getState().update({ screen: { observation: "smart" } });
    render(<ScreenTab />);

    expect(await screen.findByText(/Smart observation is not available yet/)).toBeInTheDocument();
    const interval = screen.getByLabelText<HTMLSelectElement>("Observation interval");
    expect(interval.value).toBe("1500");
    expect(within(interval).getByRole("option", { name: "Every 1.5 seconds" })).toBeInTheDocument();
  });
});
