import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import fixture from "../fixtures/native-hud-menu.json";
import { type MenuEntry } from "@/lib/tauri/menu-model";
import { nativeHudMenuRequest, openNativeHudMenu } from "@/lib/tauri/native-hud-menu";
import { TauriTransport } from "@/lib/tauri/tauri-transport";
import { setTransport } from "@/lib/tauri/transport";

const native = vi.hoisted(() => ({
  invoke: vi.fn(),
  isFocused: vi.fn(),
  setFocus: vi.fn(),
  channel: vi.fn(),
  listen: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: native.invoke, Channel: native.channel }));
vi.mock("@tauri-apps/api/event", () => ({ listen: native.listen }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "main", isFocused: native.isFocused, setFocus: native.setFocus }),
}));

const mode = { type: "mode", id: "private-domain-id" };
const entries: MenuEntry<typeof mode>[] = [
  { kind: "label", id: "summary", label: "Current session" },
  { kind: "item", id: "private-domain-id", label: "Selected mode", checked: true, action: mode },
  { kind: "separator", id: "separator" },
  {
    kind: "item",
    id: "end",
    label: "End session",
    disabled: true,
    nativeIcon: "stop",
    destructive: true,
    action: mode,
  },
];
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
let trigger: HTMLButtonElement;
beforeEach(() => {
  vi.resetAllMocks();
  Object.defineProperty(window, "__TAURI_INTERNALS__", { configurable: true, value: {} });
  setTransport(new TauriTransport());
  native.invoke.mockResolvedValue(null);
  native.isFocused.mockResolvedValue(true);
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  trigger = document.createElement("button");
  document.body.append(trigger);
  vi.spyOn(trigger, "getBoundingClientRect").mockReturnValue({
    left: 438.5,
    right: 470.5,
    bottom: 88.25,
  } as DOMRect);
});
afterEach(() => {
  trigger.remove();
  Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
  vi.restoreAllMocks();
});

describe("native HUD display-only serialization", () => {
  it("uses the same JSON fixture as Rust, not domain IDs, actions, or React nodes", () => {
    const { request, actions } = nativeHudMenuRequest(entries, fixture.position, "end");
    expect(request).toEqual(fixture);
    expect([...actions.keys()]).toEqual(["hud-1"]);
    expect(actions.get("hud-1")).toBe(mode);
    expect(JSON.stringify(request)).not.toContain("private-domain-id");
    expect(JSON.stringify(request)).not.toContain("action");
  });

  it("bounds/sanitizes native titles and does not mutate stored names or actions", () => {
    const source = `\0  Test\n${"𠮷".repeat(100)}`;
    const { request } = nativeHudMenuRequest(
      [{ kind: "item", id: "anything / private", label: source, action: mode }],
      { x: 0, y: 0 },
      "start",
    );
    const item = request.items[0];
    expect(item?.kind).toBe("item");
    if (item?.kind !== "item") throw new Error("missing item");
    expect(Array.from(item.label)).toHaveLength(80);
    expect(item.label).not.toMatch(/[\n\0]/u);
    expect(item.label.endsWith("…")).toBe(true);
    expect(source).toContain("\0");
  });

  it("rejects unsafe request bounds before IPC", () => {
    expect(() => nativeHudMenuRequest([], { x: 0, y: 0 }, "start")).toThrow();
    expect(() =>
      nativeHudMenuRequest(
        Array.from({ length: 129 }, () => entries[0]!),
        { x: 0, y: 0 },
        "start",
      ),
    ).toThrow();
    for (const v of [NaN, Infinity, -Infinity, -1, 16_385]) {
      expect(() => nativeHudMenuRequest(entries, { x: v, y: 0 }, "start")).toThrow();
      expect(() => nativeHudMenuRequest(entries, { x: 0, y: v }, "end")).toThrow();
    }
  });
});

describe("native HUD popup lifetime", () => {
  it("sends logical button-client coordinates through the real typed Tauri transport (not DPR or screen coordinates)", async () => {
    native.invoke.mockResolvedValue("hud-1");
    vi.stubGlobal("devicePixelRatio", 2);
    await expect(openNativeHudMenu(entries, trigger, "end")).resolves.toEqual({ action: mode });
    expect(native.invoke).toHaveBeenCalledExactlyOnceWith("hud_menu_popup", { request: fixture });
    expect(native.channel).not.toHaveBeenCalled();
    expect(native.listen).not.toHaveBeenCalled();
    expect(native.setFocus).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });

  it("holds one synchronous guard for both triggers through delayed tracking, with no queued reopen", async () => {
    const tracking = deferred<string | null>();
    native.invoke.mockReturnValue(tracking.promise);
    const first = openNativeHudMenu(entries, trigger, "start");
    const other = document.createElement("button");
    document.body.append(other);
    expect(openNativeHudMenu(entries, other, "end")).toBeNull();
    expect(openNativeHudMenu(entries, trigger, "start")).toBeNull();
    expect(native.invoke).toHaveBeenCalledOnce();
    tracking.resolve("hud-1");
    await expect(first).resolves.toEqual({ action: mode });
    native.invoke.mockResolvedValue(null);
    await expect(openNativeHudMenu(entries, trigger, "start")).resolves.toBeNull();
    expect(native.invoke).toHaveBeenCalledTimes(2);
    other.remove();
  });

  it.each([null, "hud-0", "hud-2", "hud-3", "unknown", "private-domain-id"])(
    "ignores cancellation or nonselectable returned id %s",
    async (id) => {
      native.invoke.mockResolvedValue(id);
      await expect(openNativeHudMenu(entries, trigger, "start")).resolves.toBeNull();
    },
  );

  it("unlocks after native creation/tracking rejection and permits retry without registrations", async () => {
    const tracking = deferred<string | null>();
    native.invoke.mockReturnValueOnce(tracking.promise);
    const first = openNativeHudMenu(entries, trigger, "start");
    expect(openNativeHudMenu(entries, trigger, "end")).toBeNull();
    tracking.reject(new Error("native creation failed"));
    await expect(first).rejects.toMatchObject({ message: "native creation failed" });
    for (let i = 0; i < 30; i++) await openNativeHudMenu(entries, trigger, "start");
    expect(native.invoke).toHaveBeenCalledTimes(31);
    expect(native.listen).not.toHaveBeenCalled();
    expect(native.channel).not.toHaveBeenCalled();
  });

  it("unlocks when serialization fails before IPC", async () => {
    await expect(openNativeHudMenu([], trigger, "start")).rejects.toThrow("item count");
    expect(native.invoke).not.toHaveBeenCalled();
    await expect(openNativeHudMenu(entries, trigger, "start")).resolves.toBeNull();
  });

  it("does not abandon native tracking when the original trigger is disconnected", async () => {
    const tracking = deferred<string | null>();
    native.invoke.mockReturnValueOnce(tracking.promise);
    trigger.focus();
    const focus = vi.spyOn(trigger, "focus");
    const first = openNativeHudMenu(entries, trigger, "start");
    trigger.remove();
    expect(openNativeHudMenu(entries, trigger, "start")).toBeNull();
    tracking.resolve(null);
    await first;
    expect(focus).not.toHaveBeenCalled();
    document.body.append(trigger);
    await expect(openNativeHudMenu(entries, trigger, "start")).resolves.toBeNull();
  });

  it("restores only existing DOM trigger focus before the caller can open Settings", async () => {
    trigger.focus();
    const focus = vi.spyOn(trigger, "focus");
    native.invoke.mockResolvedValue("hud-1");
    const selection = await openNativeHudMenu(entries, trigger, "start");
    expect(selection).toEqual({ action: mode });
    expect(native.isFocused).toHaveBeenCalledOnce();
    expect(focus).toHaveBeenCalledExactlyOnceWith({ preventScroll: true });
    expect(native.setFocus).not.toHaveBeenCalled();
  });

  it("holds the guard during the read-only focus query and unlocks if that query fails", async () => {
    trigger.focus();
    const query = deferred<boolean>();
    native.isFocused.mockReturnValueOnce(query.promise);
    const first = openNativeHudMenu(entries, trigger, "start");
    await vi.waitFor(() => expect(native.isFocused).toHaveBeenCalledOnce());
    expect(openNativeHudMenu(entries, trigger, "end")).toBeNull();
    query.reject(new Error("window disappeared"));
    await expect(first).resolves.toBeNull();
    await expect(openNativeHudMenu(entries, trigger, "start")).resolves.toBeNull();
  });

  it.each(["document", "native", "pointer"])(
    "does not restore focus after %s focus loss/nonfocused activation",
    async (kind) => {
      if (kind !== "pointer") trigger.focus();
      const focus = vi.spyOn(trigger, "focus");
      const tracking = deferred<string | null>();
      native.invoke.mockReturnValueOnce(tracking.promise);
      const first = openNativeHudMenu(entries, trigger, "start");
      if (kind === "document") vi.mocked(document.hasFocus).mockReturnValue(false);
      if (kind === "native") native.isFocused.mockResolvedValue(false);
      tracking.resolve(null);
      await first;
      expect(focus).not.toHaveBeenCalled();
      expect(native.setFocus).not.toHaveBeenCalled();
    },
  );

  it.each(["document", "detached", "disabled", "other control"])(
    "rechecks %s state after a delayed native focus reply",
    async (kind) => {
      trigger.focus();
      const restore = vi.spyOn(trigger, "focus");
      const query = deferred<boolean>();
      native.isFocused.mockReturnValueOnce(query.promise);
      const first = openNativeHudMenu(entries, trigger, "start");
      await vi.waitFor(() => expect(native.isFocused).toHaveBeenCalledOnce());
      const other = document.createElement("input");
      document.body.append(other);
      if (kind === "document") vi.mocked(document.hasFocus).mockReturnValue(false);
      if (kind === "detached") trigger.remove();
      if (kind === "disabled") trigger.disabled = true;
      if (kind === "other control") other.focus();
      query.resolve(true);
      await first;
      expect(restore).not.toHaveBeenCalled();
      other.remove();
    },
  );

  it("releases the guard even if the initial DOM focus read throws", async () => {
    vi.mocked(document.hasFocus).mockImplementationOnce(() => {
      throw new Error("document gone");
    });
    await expect(openNativeHudMenu(entries, trigger, "start")).rejects.toThrow("document gone");
    expect(native.invoke).not.toHaveBeenCalled();
    await expect(openNativeHudMenu(entries, trigger, "start")).resolves.toBeNull();
  });

  it("never calls the command for browser fallback, disabled or detached buttons", () => {
    trigger.disabled = true;
    expect(openNativeHudMenu(entries, trigger, "start")).toBeNull();
    trigger.disabled = false;
    trigger.remove();
    expect(openNativeHudMenu(entries, trigger, "start")).toBeNull();
    document.body.append(trigger);
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    expect(openNativeHudMenu(entries, trigger, "start")).toBeNull();
    expect(native.invoke).not.toHaveBeenCalled();
  });
});
