import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { ResponseThread } from "@/features/hud/ResponseThread";
import { followScrollTop, TURN_TOP_MARGIN_PX } from "@/features/hud/thread-scroll";
import { useChatStore } from "@/stores/chatStore";
import { makeResponse, setupMockApp } from "./helpers";

describe("followScrollTop (UX-027)", () => {
  const view = (scrollTop: number, scrollHeight: number) => ({ scrollTop, scrollHeight, clientHeight: 400 });

  it("follows the bottom while the turn's first line stays in view", () => {
    expect(followScrollTop(view(100, 700), 520)).toBe(300);
  });

  it("stops at the turn's first line once the turn outgrows the viewport", () => {
    expect(followScrollTop(view(300, 1500), 520)).toBe(520 - TURN_TOP_MARGIN_PX);
  });

  it("keeps following the bottom for a reader already below that line", () => {
    expect(followScrollTop(view(900, 1500), 520)).toBe(1100);
  });
});

/** Lays out the thread: the newest turn starts at `turnTop`, the content is `height` tall. */
function layout(region: HTMLElement, box: { turnTop: number; height: number }) {
  Object.defineProperty(region, "clientHeight", { configurable: true, get: () => 400 });
  Object.defineProperty(region, "scrollHeight", { configurable: true, get: () => box.height });
  region.scrollTo = vi.fn((options?: ScrollToOptions | number) => {
    if (typeof options === "object") region.scrollTop = options.top ?? region.scrollTop;
  }) as typeof region.scrollTo;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    const isLastTurn = this === region.firstElementChild?.lastElementChild;
    return DOMRect.fromRect({ y: isLastTurn ? box.turnTop - region.scrollTop : 0, height: 0 });
  });
}

const nextFrame = () => act(() => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));

describe("ResponseThread follow (UX-027)", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("keeps a long streaming answer's first line in view and hands over to the ↓ button", async () => {
    act(() => useChatStore.getState().showResponse(makeResponse({ content: "Earlier answer." })));
    render(
      <TooltipProvider>
        <ResponseThread onRetry={() => {}} onRegenerate={() => {}} />
      </TooltipProvider>,
    );
    const region = screen.getByRole("region", { name: "Response" });
    const box = { turnTop: 0, height: 500 };
    layout(region, box);
    await nextFrame();

    // A new turn starts below the old one and is still short: follow the bottom.
    let generation = 0;
    act(() => {
      generation = useChatStore.getState().begin("Why is the build slow?", undefined, { phase: "thinking" });
    });
    Object.assign(box, { turnTop: 520, height: 600 });
    await nextFrame();
    expect(region.scrollTop).toBe(200);

    // It streams past the viewport: stop at its first line, not the bottom.
    box.height = 1200;
    act(() => useChatStore.getState().complete(generation, makeResponse({ content: "Say: the cache is cold." })));
    await nextFrame();
    expect(region.scrollTop).toBe(520 - TURN_TOP_MARGIN_PX);
    expect(screen.getByRole("button", { name: "Scroll to bottom" })).toBeInTheDocument();
  });
});
