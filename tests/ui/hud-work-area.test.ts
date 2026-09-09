import { act, renderHook, waitFor } from "@testing-library/react";
import type { Monitor } from "@tauri-apps/api/window";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  native: true,
  monitor: vi.fn(),
  moved: vi.fn(),
  scaled: vi.fn(),
}));
vi.mock("@/lib/tauri/transport", () => ({ hasTauriRuntime: () => mocks.native }));
vi.mock("@tauri-apps/api/window", () => ({
  currentMonitor: mocks.monitor,
  getCurrentWindow: () => ({ onMoved: mocks.moved, onScaleChanged: mocks.scaled }),
}));

import { useHudWorkArea } from "@/features/hud/useHudWorkArea";
import { currentWorkArea } from "@/lib/tauri/work-area";

function monitor(physicalHeight: number, scale = 2): Monitor {
  return {
    scaleFactor: scale,
    workArea: {
      size: {
        toLogical: vi.fn((factor: number) => ({ width: 1600 / factor, height: physicalHeight / factor })),
      },
    },
  } as unknown as Monitor;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("HUD work-area bounds", () => {
  beforeEach(() => {
    mocks.native = true;
    mocks.monitor.mockReset().mockResolvedValue(monitor(720));
    mocks.moved.mockReset().mockResolvedValue(() => {});
    mocks.scaled.mockReset().mockResolvedValue(() => {});
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("converts the work area by its own monitor scale exactly once", async () => {
    vi.stubGlobal("devicePixelRatio", 3);
    vi.stubGlobal("innerHeight", 175);
    const display = monitor(720, 2);
    mocks.monitor.mockResolvedValue(display);
    const { result } = renderHook(useHudWorkArea);
    await waitFor(() => {
      expect(result.current).toEqual({ width: 800, height: 360 });
    });
    expect(display.workArea.size.toLogical).toHaveBeenCalledWith(2);
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(mocks.monitor).toHaveBeenCalledTimes(1); // native auto-height is not the cap
    expect(result.current.height).toBe(360);
  });

  it("drops a stale monitor lookup after moving between displays", async () => {
    const first = deferred<Monitor>();
    const second = deferred<Monitor>();
    mocks.monitor.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const { result } = renderHook(useHudWorkArea);
    const onMoved = mocks.moved.mock.calls[0]![0] as () => void;
    act(() => {
      onMoved();
    });
    await act(async () => {
      second.resolve(monitor(600, 2));
    });
    expect(result.current.height).toBe(300);
    await act(async () => {
      first.resolve(monitor(1800, 2));
    });
    expect(result.current.height).toBe(300);
  });

  it("cleans up both registered and late-arriving native subscriptions", async () => {
    const late = deferred<() => void>();
    const unlistenMoved = vi.fn();
    const unlistenScaled = vi.fn();
    mocks.moved.mockReturnValueOnce(late.promise);
    mocks.scaled.mockResolvedValueOnce(unlistenScaled);
    const view = renderHook(useHudWorkArea);
    await waitFor(() => {
      expect(view.result.current.height).toBe(360);
    });
    view.unmount();
    expect(unlistenScaled).toHaveBeenCalledOnce();
    await act(async () => {
      late.resolve(unlistenMoved);
    });
    expect(unlistenMoved).toHaveBeenCalledOnce();
  });

  it("preserves fractional logical work dimensions on a non-integer scale", async () => {
    const display = monitor(961, 1.5);
    mocks.monitor.mockResolvedValue(display);
    expect(await currentWorkArea()).toEqual({ width: 1600 / 1.5, height: 961 / 1.5 });
    expect(display.workArea.size.toLogical).toHaveBeenCalledExactlyOnceWith(1.5);
  });

  it("rejects invalid monitor dimensions instead of inventing a one-pixel cap", async () => {
    for (const display of [monitor(600, 0), monitor(600, Number.NaN), monitor(0), monitor(Number.NaN)]) {
      mocks.monitor.mockResolvedValue(display);
      expect(await currentWorkArea()).toBeNull();
    }
  });

  it("uses an independent screen fallback during monitor failure and recovers on focus", async () => {
    vi.stubGlobal("innerHeight", 175);
    vi.stubGlobal("screen", { availWidth: 1024, availHeight: 768 });
    mocks.monitor.mockRejectedValueOnce(new Error("monitor detached"));
    const { result } = renderHook(useHudWorkArea);
    await act(async () => {});
    expect(result.current).toEqual({ width: 1024, height: 768 });
    mocks.monitor.mockResolvedValue(monitor(600, 1));
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
    });
    expect(result.current).toEqual({ width: 1600, height: 600 });
  });

  it("suppresses unchanged work-area snapshots and ignores late callbacks after disposal", async () => {
    const view = renderHook(useHudWorkArea);
    await waitFor(() => {
      expect(view.result.current.height).toBe(360);
    });
    const snapshot = view.result.current;
    const onMoved = mocks.moved.mock.calls[0]![0] as () => void;
    const onScaled = mocks.scaled.mock.calls[0]![0] as () => void;
    await act(async () => {
      onMoved();
    });
    expect(view.result.current).toBe(snapshot);
    const calls = mocks.monitor.mock.calls.length;
    view.unmount();
    await act(async () => {
      onMoved();
      onScaled();
      window.dispatchEvent(new Event("focus"));
    });
    expect(mocks.monitor).toHaveBeenCalledTimes(calls);
  });

  it("refreshes a changed screen work area without depending on native innerHeight", async () => {
    vi.stubGlobal("screen", { availWidth: 1600, availHeight: 900 });
    const view = renderHook(useHudWorkArea);
    await waitFor(() => {
      expect(view.result.current.height).toBe(360);
    });
    mocks.monitor.mockResolvedValue(monitor(900, 1));
    vi.stubGlobal("screen", { availWidth: 1600, availHeight: 450 });
    await act(async () => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(view.result.current.height).toBe(900); // native work area, not innerHeight or screen guess
    expect(mocks.monitor).toHaveBeenCalledTimes(2);
  });

  it("uses viewport bounds only in the non-auto-sized browser preview", async () => {
    mocks.native = false;
    vi.stubGlobal("innerHeight", 360);
    const { result } = renderHook(useHudWorkArea);
    expect(result.current.height).toBe(360);
    vi.stubGlobal("innerHeight", 600);
    await act(async () => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(result.current.height).toBe(600);
    expect(mocks.monitor).not.toHaveBeenCalled();
  });
});
