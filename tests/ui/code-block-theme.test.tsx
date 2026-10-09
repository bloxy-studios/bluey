import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { CodeBlock } from "@/features/hud/CodeBlock";
import { highlightCode } from "@/features/hud/highlighter";

vi.mock("@/features/hud/highlighter", () => ({
  highlightCode: vi.fn(async (code: string, _language: string | undefined, theme: string) => `<pre data-theme-used="${theme}">${code}</pre>`),
}));

describe("CodeBlock theming (UX-005)", () => {
  afterEach(() => {
    delete document.documentElement.dataset.theme;
  });

  it("paints the themed code surface, not a dark-only literal", () => {
    document.documentElement.dataset.theme = "light";
    const { container } = render(<CodeBlock code={"const a = 1;"} language="ts" />);
    const surface = container.querySelector(".code-block");
    expect(surface).toHaveClass("bg-code-bg");
    expect(surface?.className).not.toMatch(/#0d0d0d|bg-white\//);
  });

  it("re-highlights with the new theme when the appearance changes", async () => {
    document.documentElement.dataset.theme = "dark";
    render(<CodeBlock code={"x"} language="ts" />);
    await waitFor(() => expect(screen.getByText("x")).toHaveAttribute("data-theme-used", "dark"));

    document.documentElement.dataset.theme = "light";
    await waitFor(() => expect(screen.getByText("x")).toHaveAttribute("data-theme-used", "light"));
    expect(vi.mocked(highlightCode)).toHaveBeenLastCalledWith("x", "ts", "light");
  });
});
