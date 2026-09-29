import { render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import ScreenTab from "@/features/settings/tabs/ScreenTab";
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
