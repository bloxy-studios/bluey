import { act, render } from "@testing-library/react";
import { useRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  HUD_FRAME_INSETS,
  hudFrameWidth,
  hudSurfaceMaxHeight,
  measuredFrameHeight,
} from "@/features/hud/geometry";
import { useAutoHeight } from "@/features/hud/useAutoHeight";
import { usePanelStore } from "@/stores/panelStore";

class Observer implements ResizeObserver {
  static instances: Observer[] = [];
  target: Element | null = null;
  observe = vi.fn((target: Element) => {
    this.target = target;
  });
  unobserve = vi.fn();
  disconnect = vi.fn();
  constructor(private readonly callback: ResizeObserverCallback) {
    Observer.instances.push(this);
  }
  resize(height: number, contentHeight = height - 64) {
    this.callback(
      [
        {
          target: this.target,
          borderBoxSize: [{ blockSize: height, inlineSize: 754 }],
          contentRect: { height: contentHeight },
        } as unknown as ResizeObserverEntry,
      ],
      this,
    );
  }
}

function Measurement({
  expanded = false,
  workAreaKey = "large",
}: {
  expanded?: boolean;
  workAreaKey?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useAutoHeight(ref, expanded, workAreaKey);
  return <div ref={ref} data-testid="frame" style={{ padding: "24px 32px 40px" }} />;
}

describe("HUD frame measurement", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    Observer.instances = [];
    vi.stubGlobal("ResizeObserver", Observer);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ height: 175.25 } as DOMRect);
    vi.spyOn(usePanelStore.getState(), "setExpanded").mockResolvedValue(true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("measures the border-box including outer insets, without multiplying by Retina scale", () => {
    vi.stubGlobal("devicePixelRatio", 2);
    const view = render(<Measurement />);
    const observer = Observer.instances[0]!;
    expect(observer.observe).toHaveBeenCalledWith(view.getByTestId("frame"), { box: "border-box" });
    act(() => {
      vi.advanceTimersByTime(80);
    });
    expect(usePanelStore.getState().setExpanded).toHaveBeenLastCalledWith(false, 176);

    // A smaller contentRect must not drop the two surface borders or frame padding.
    act(() => {
      observer.resize(175.25, 109.25);
      vi.advanceTimersByTime(80);
    });
    expect(usePanelStore.getState().setExpanded).toHaveBeenCalledTimes(1);
    expect(HUD_FRAME_INSETS.top + HUD_FRAME_INSETS.bottom).toBe(64);
    expect(hudFrameWidth(690)).toBe(754);
  });

  it("grows and shrinks live transcript rows while expanded remains false", () => {
    render(<Measurement />);
    const observer = Observer.instances[0]!;
    act(() => {
      vi.advanceTimersByTime(80);
    });
    for (const height of [232, 286, 232, 175]) {
      act(() => {
        observer.resize(height);
        vi.advanceTimersByTime(80);
      });
      expect(usePanelStore.getState().setExpanded).toHaveBeenLastCalledWith(false, height);
    }
    expect(usePanelStore.getState().setExpanded).toHaveBeenCalledTimes(5);
  });

  it("coalesces duplicate reports without starving continuously arriving content", () => {
    render(<Measurement expanded />);
    const observer = Observer.instances[0]!;
    for (const height of [200, 230, 260, 290]) {
      act(() => {
        observer.resize(height);
        vi.advanceTimersByTime(20);
      });
    }
    expect(usePanelStore.getState().setExpanded).toHaveBeenCalledTimes(1);
    expect(usePanelStore.getState().setExpanded).toHaveBeenLastCalledWith(true, 290);
    act(() => {
      observer.resize(290);
      vi.advanceTimersByTime(80);
    });
    expect(usePanelStore.getState().setExpanded).toHaveBeenCalledTimes(1);
  });

  it("cancels stale pending measurements on mode change and ignores disconnected observers", () => {
    const view = render(<Measurement expanded />);
    const old = Observer.instances[0]!;
    act(() => {
      old.resize(600);
    });
    view.rerender(<Measurement expanded={false} />);
    act(() => {
      old.resize(620);
      vi.advanceTimersByTime(80);
    });
    expect(old.disconnect).toHaveBeenCalledOnce();
    expect(usePanelStore.getState().setExpanded).toHaveBeenCalledExactlyOnceWith(false, 176);

    const current = Observer.instances[1]!;
    act(() => {
      current.resize(250);
    });
    view.unmount();
    act(() => {
      current.resize(300);
      vi.advanceTimersByTime(80);
    });
    expect(current.disconnect).toHaveBeenCalledOnce();
    expect(usePanelStore.getState().setExpanded).toHaveBeenCalledTimes(1);
  });

  it("rechecks an unchanged height after work-area changes, not native viewport height changes", () => {
    const view = render(<Measurement />);
    act(() => {
      vi.advanceTimersByTime(80);
    });
    act(() => {
      window.dispatchEvent(new Event("resize"));
      vi.advanceTimersByTime(80);
    });
    expect(usePanelStore.getState().setExpanded).toHaveBeenCalledTimes(1);
    view.rerender(<Measurement workAreaKey="small" />);
    act(() => {
      vi.advanceTimersByTime(80);
    });
    expect(usePanelStore.getState().setExpanded).toHaveBeenCalledTimes(2);
  });

  it("falls back to the same border-box when WebKit omits borderBoxSize", () => {
    const element = document.createElement("div");
    const entry = { contentRect: { height: 109.25 } } as ResizeObserverEntry;
    expect(measuredFrameHeight(element, entry)).toBe(176);
    vi.spyOn(element, "getBoundingClientRect").mockReturnValue({ height: 0 } as DOMRect);
    expect(measuredFrameHeight(element, entry)).toBeNull();
  });

  it("drops a pending height when a newer observation is zero or invalid", () => {
    render(<Measurement />);
    const observer = Observer.instances[0]!;
    act(() => {
      observer.resize(300);
      observer.resize(0);
      vi.advanceTimersByTime(80);
    });
    expect(usePanelStore.getState().setExpanded).not.toHaveBeenCalled();
    act(() => {
      observer.resize(240);
      observer.resize(Number.NaN);
      vi.advanceTimersByTime(80);
    });
    expect(usePanelStore.getState().setExpanded).not.toHaveBeenCalled();
    act(() => {
      observer.resize(240);
      vi.advanceTimersByTime(80);
    });
    expect(usePanelStore.getState().setExpanded).toHaveBeenCalledExactlyOnceWith(false, 240);
  });

  it("still reports the initial border-box without ResizeObserver", () => {
    vi.stubGlobal("ResizeObserver", undefined);
    render(<Measurement />);
    act(() => {
      vi.advanceTimersByTime(80);
    });
    expect(usePanelStore.getState().setExpanded).toHaveBeenCalledExactlyOnceWith(false, 176);
  });

  it("retries a static measurement once on transient native failure, not forever", async () => {
    const resize = vi.mocked(usePanelStore.getState().setExpanded).mockResolvedValue(false);
    const view = render(<Measurement />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(80);
    });
    expect(resize).toHaveBeenCalledTimes(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(800);
    });
    expect(resize).toHaveBeenCalledTimes(2);
    expect(resize).toHaveBeenLastCalledWith(false, 176);
    // A later real observer notification is still retryable (not cached as success).
    resize.mockResolvedValue(true);
    await act(async () => {
      Observer.instances[0]!.resize(175.25);
      await vi.advanceTimersByTimeAsync(80);
    });
    expect(resize).toHaveBeenCalledTimes(3);
    view.unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(800);
    });
    expect(resize).toHaveBeenCalledTimes(3);
  });

  it("uses work-area height minus insets and still allows growth after collapse", () => {
    expect(hudSurfaceMaxHeight(950)).toBe(620);
    expect(hudSurfaceMaxHeight(360)).toBe(296);
    // An auto-sized 175px native viewport must not become the next growth cap.
    vi.stubGlobal("innerHeight", 175);
    expect(hudSurfaceMaxHeight(950)).toBe(620);
    expect(hudSurfaceMaxHeight(0)).toBe(620);
    expect(hudSurfaceMaxHeight(Number.NaN)).toBe(620);
    expect(hudSurfaceMaxHeight(40)).toBe(1);
  });
});
