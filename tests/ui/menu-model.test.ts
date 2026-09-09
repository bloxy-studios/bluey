import { describe, expect, it } from "vitest";

import { menuLabel, type MenuEntry } from "@/lib/tauri/menu-model";

describe("menu presentation data", () => {
  it("normalizes multiline labels without mutating the stored name", () => {
    const original = "  Interview\n  Design\t Review  ";
    expect(menuLabel(original)).toBe("Interview Design Review");
    expect(original).toContain("\n");
  });

  it("bounds display labels to 80 Unicode code points without splitting surrogate pairs", () => {
    expect(menuLabel("a".repeat(80))).toHaveLength(80);
    expect(menuLabel("a".repeat(81))).toBe(`${"a".repeat(79)}…`);
    const long = menuLabel("𠮷".repeat(90));
    expect(Array.from(long)).toHaveLength(80);
    expect(long).toBe(`${"𠮷".repeat(79)}…`);
  });

  it("keeps typed action identity separate from presentation/selection", () => {
    type Action = { kind: "mode"; id: string } | { kind: "manage" };
    const action: Action = { kind: "mode", id: "unchanged-id" };
    const menu: MenuEntry<Action>[] = [
      { kind: "label", id: "heading", label: "Modes" },
      { kind: "item", id: "first", label: "First", checked: true, action },
      { kind: "separator", id: "separator" },
      { kind: "item", id: "manage", label: "Manage", action: { kind: "manage" } },
    ];
    const entry = menu[1];
    expect(entry?.kind).toBe("item");
    if (entry?.kind === "item") expect(entry.action).toBe(action);
  });
});
