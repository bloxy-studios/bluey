import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { bluey } from "@/lib/tauri/api";
import type { PanelState } from "@/lib/types";
import { usePanelStore } from "@/stores/panelStore";

const initial: PanelState = {
  visible: true,
  pinned: false,
  expanded: false,
  x: 300,
  y: 50,
  width: 754,
  height: 175,
  opacity: 0.92,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("panel resize sequencing", () => {
  beforeEach(() => {
    usePanelStore.getState().applyRemote(initial);
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("serializes native resizes and sends only the latest pending measurement", async () => {
    const first = deferred<PanelState>();
    const resize = vi
      .spyOn(bluey.panel, "setExpanded")
      .mockReturnValueOnce(first.promise)
      .mockResolvedValueOnce({ ...initial, height: 230 });
    const seen: number[] = [];
    const unsubscribe = usePanelStore.subscribe((store) => {
      if (store.state) seen.push(store.state.height);
    });

    const expanding = usePanelStore.getState().setExpanded(true, 600);
    const collapsed = usePanelStore.getState().setExpanded(false, 175);
    const listening = usePanelStore.getState().setExpanded(false, 230);
    expect(resize).toHaveBeenCalledTimes(1);
    first.resolve({ ...initial, expanded: true, height: 600 });
    await Promise.all([expanding, collapsed, listening]);
    unsubscribe();

    expect(resize.mock.calls).toEqual([
      [{ expanded: true, height: 600 }],
      [{ expanded: false, height: 230 }],
    ]);
    expect(seen).toEqual([230]); // no stale expanded snapshot on the way to the final intent
    expect(usePanelStore.getState().state).toEqual({ ...initial, height: 230 });
  });

  it("suppresses duplicates in flight, including a return to the active intent", async () => {
    const response = deferred<PanelState>();
    const resize = vi.spyOn(bluey.panel, "setExpanded").mockReturnValue(response.promise);
    const calls = [
      usePanelStore.getState().setExpanded(true, 500),
      usePanelStore.getState().setExpanded(false, 175),
      usePanelStore.getState().setExpanded(true, 500),
      usePanelStore.getState().setExpanded(true, 500),
    ];
    response.resolve({ ...initial, expanded: true, height: 500 });
    await Promise.all(calls);
    expect(resize).toHaveBeenCalledExactlyOnceWith({ expanded: true, height: 500 });
  });

  it("does not overwrite a newer keyboard-move/opacity event with an old command response", async () => {
    const response = deferred<PanelState>();
    vi.spyOn(bluey.panel, "setExpanded").mockReturnValue(response.promise);
    const resizing = usePanelStore.getState().setExpanded(true, 500);
    const moved = { ...initial, x: 324, y: 74, expanded: true, height: 500, opacity: 0.4, pinned: true };
    usePanelStore.getState().applyRemote(moved);
    response.resolve({ ...initial, expanded: true, height: 500 });
    await resizing;
    expect(usePanelStore.getState().state).toEqual(moved);
  });

  it("does not overwrite a panel event that arrives during the initial load", async () => {
    const response = deferred<PanelState>();
    vi.spyOn(bluey.panel, "getState").mockReturnValue(response.promise);
    const loading = usePanelStore.getState().load();
    const moved = { ...initial, x: 324, y: 74 };
    usePanelStore.getState().applyRemote(moved);
    response.resolve(initial);
    await loading;
    expect(usePanelStore.getState().state).toEqual(moved);
  });

  it("does not let a slow initial load overwrite a resize reply when no event arrives", async () => {
    const response = deferred<PanelState>();
    vi.spyOn(bluey.panel, "getState").mockReturnValue(response.promise);
    const grown = { ...initial, expanded: true, height: 500 };
    vi.spyOn(bluey.panel, "setExpanded").mockResolvedValue(grown);
    const loading = usePanelStore.getState().load();
    await usePanelStore.getState().setExpanded(true, 500);
    response.resolve(initial);
    await loading;
    expect(usePanelStore.getState().state).toEqual(grown);
  });

  it("discards an old load even when a resize started but has not replied yet", async () => {
    const response = deferred<PanelState>();
    vi.spyOn(bluey.panel, "setExpanded").mockReturnValue(response.promise);
    vi.spyOn(bluey.panel, "getState").mockResolvedValue({ ...initial, x: 0 });
    const resizing = usePanelStore.getState().setExpanded(true, 500);
    await usePanelStore.getState().load();
    expect(usePanelStore.getState().state).toEqual(initial);
    response.resolve({ ...initial, expanded: true, height: 500 });
    await resizing;
    expect(usePanelStore.getState().state?.height).toBe(500);
  });

  it("accepts only the newest of overlapping loads", async () => {
    const first = deferred<PanelState>();
    const second = deferred<PanelState>();
    vi.spyOn(bluey.panel, "getState").mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const loads = [usePanelStore.getState().load(), usePanelStore.getState().load()];
    const moved = { ...initial, x: 324 };
    second.resolve(moved);
    await loads[1];
    first.resolve(initial);
    await loads[0];
    expect(usePanelStore.getState().state).toEqual(moved);
  });

  it("recovers from a synchronous transport throw as well as a rejected promise", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const resize = vi
      .spyOn(bluey.panel, "setExpanded")
      .mockImplementationOnce(() => {
        throw new Error("transport not ready");
      })
      .mockResolvedValueOnce({ ...initial, height: 230 });
    expect(await usePanelStore.getState().setExpanded(false, 230)).toBe(false);
    expect(await usePanelStore.getState().setExpanded(false, 230)).toBe(true);
    expect(resize).toHaveBeenCalledTimes(2);
    expect(usePanelStore.getState().state?.height).toBe(230);
  });

  it("recovers after a failed resize instead of leaving the queue locked", async () => {
    const warning = vi.spyOn(console, "warn").mockImplementation(() => {});
    const resize = vi
      .spyOn(bluey.panel, "setExpanded")
      .mockRejectedValueOnce(new Error("window unavailable"))
      .mockResolvedValueOnce({ ...initial, height: 230 });
    await usePanelStore.getState().setExpanded(false, 230);
    expect(usePanelStore.getState().state).toEqual(initial);
    await usePanelStore.getState().setExpanded(false, 230);
    expect(resize).toHaveBeenCalledTimes(2);
    expect(warning).toHaveBeenCalledOnce();
    expect(usePanelStore.getState().state?.height).toBe(230);
  });
});
