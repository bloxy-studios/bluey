import { readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it, vi } from "vitest";

import { HUD_FRAME_INSETS, hudFrameWidth } from "@/features/hud/geometry";
import { MockTransport } from "@/lib/tauri/mock";

const ROOT = resolve(__dirname, "../..");
const config = JSON.parse(readFileSync(resolve(ROOT, "src-tauri/tauri.conf.json"), "utf8")) as {
  app: {
    windows: Array<{
      label: string;
      width: number;
      height: number;
      minWidth: number;
      minHeight: number;
      resizable: boolean;
    }>;
  };
};
const main = config.app.windows.find((window) => window.label === "main")!;

describe("HUD frame contract across browser mock, CSS and native config", () => {
  it("keeps each shadow inset identical to the pure native geometry", () => {
    const source = readFileSync(resolve(ROOT, "src-tauri/crates/bluey-protocols/src/panel.rs"), "utf8");
    for (const [side, value] of Object.entries(HUD_FRAME_INSETS)) {
      const native = new RegExp(`pub const FRAME_INSET_${side.toUpperCase()}: f64 = ([0-9.]+);`).exec(source);
      expect(Number(native?.[1]), side).toBe(value);
    }
    expect(main.width).toBe(hudFrameWidth(690));
    expect(main.height).toBe(56 + 52 + 3 + HUD_FRAME_INSETS.top + HUD_FRAME_INSETS.bottom);
    expect(main.minWidth).toBe(hudFrameWidth(420));
    expect(main.minHeight).toBe(main.height);
    expect(main.resizable).toBe(false);
  });

  it("starts the mock at the same frame dimensions and honors idle transcript measurements", async () => {
    const mock = new MockTransport({ levelTicks: false });
    const initial = await mock.invoke("panel_get_state", undefined);
    expect([initial.width, initial.height]).toEqual([main.width, main.height]);
    const changed = vi.fn();
    const unlisten = await mock.listen("panel.state", changed);
    const grown = await mock.invoke("panel_set_expanded", { expanded: false, height: 286.25 });
    expect(grown.height).toBe(287);
    expect(grown.expanded).toBe(false);
    expect(grown.width).toBe(main.width);
    expect(await mock.invoke("panel_set_expanded", { expanded: false, height: 286.25 })).toBe(grown);
    expect(changed).toHaveBeenCalledOnce();
    expect((await mock.invoke("panel_set_expanded", { expanded: false })).height).toBe(main.height);
    expect((await mock.invoke("panel_set_expanded", { expanded: true })).height).toBe(544);
    expect((await mock.invoke("panel_set_expanded", { expanded: false, height: Number.NaN })).height).toBe(
      main.height,
    );
    unlisten();
  });

  it("keeps direct native window imports out of HUD feature modules", () => {
    const directory = resolve(ROOT, "src/features/hud");
    for (const file of readdirSync(directory).filter((name) => /\.tsx?$/.test(name))) {
      const source = readFileSync(resolve(directory, file), "utf8");
      expect(source, file).not.toMatch(/(?:from\s*|import\s*\()["']@tauri-apps\//);
    }
  });
});
