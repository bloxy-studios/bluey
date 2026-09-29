import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ErrorBoundary } from "@/components/ui/ErrorBoundary";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { HudPanel } from "@/features/hud/HudPanel";
import type { BlueyResponse } from "@/lib/types";
import { useChatStore } from "@/stores/chatStore";
import { setEngine } from "@/stores/engine";
import { FakeEngine, makeResponse, setupMockApp } from "./helpers";

// A renderer that throws on one poisoned answer, like a markdown edge case would.
vi.mock("@/features/hud/ResponseView", () => ({
  ResponseView: ({ response }: { response: BlueyResponse }) => {
    if (response.content === "POISON") throw new Error("render failed");
    return <p>{response.content}</p>;
  },
}));

function Thrower(): never {
  throw new Error("boom");
}

describe("error boundaries (UX-038)", () => {
  beforeEach(async () => {
    await setupMockApp();
    setEngine(new FakeEngine());
    // React logs caught render errors; keep the test output readable.
    vi.spyOn(console, "error").mockImplementation(() => undefined);
  });

  it("renders a compact Reload fallback instead of a blank window", () => {
    render(
      <ErrorBoundary>
        <Thrower />
      </ErrorBoundary>,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Something went wrong.");
    expect(screen.getByRole("button", { name: "Reload" })).toBeInTheDocument();
  });

  it("keeps the HUD and the other turns when one turn fails to render", () => {
    render(
      <TooltipProvider>
        <HudPanel />
      </TooltipProvider>,
    );
    act(() => {
      for (const content of ["first answer", "POISON", "third answer"]) {
        const generation = useChatStore.getState().begin(`asked for ${content}`);
        useChatStore.getState().complete(generation, makeResponse({ content }));
      }
    });

    expect(screen.getByText("first answer")).toBeInTheDocument();
    expect(screen.getByText("third answer")).toBeInTheDocument();
    expect(screen.getByText("This answer couldn’t be displayed.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start audio session" })).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Ask follow-up" })).toBeInTheDocument();
  });
});
