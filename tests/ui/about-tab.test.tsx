import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import AboutTab from "@/features/settings/tabs/AboutTab";
import { openExternal } from "@/lib/utils/open-external";
import { setupMockApp } from "./helpers";

vi.mock("@/lib/utils/open-external", () => ({ openExternal: vi.fn(async () => {}) }));

describe("AboutTab (UX-043)", () => {
  beforeEach(async () => {
    await setupMockApp();
    vi.mocked(openExternal).mockClear();
  });

  it("sends Help to the README and Support to the GitHub issues", async () => {
    const user = userEvent.setup();
    render(<AboutTab />);

    await user.click(screen.getByRole("button", { name: /^Open$/ }));
    await user.click(screen.getByRole("button", { name: /^Open issue/ }));

    expect(vi.mocked(openExternal).mock.calls).toEqual([
      ["https://github.com/bloxy-studios/bluey#readme"],
      ["https://github.com/bloxy-studios/bluey/issues"],
    ]);
  });
});
