import type { HudMenuIcon } from "./hud-menu-types";

/** Platform-neutral menu data. No native resources or action/error ownership here. */
export type MenuEntry<Action> =
  | {
      kind: "item";
      id: string;
      label: string;
      action: Action;
      checked?: boolean;
      disabled?: boolean;
      destructive?: boolean;
      nativeIcon?: HudMenuIcon;
    }
  | { kind: "label"; id: string; label: string }
  | { kind: "separator"; id: string };

/** Single-line, bounded display labels; never modify the underlying mode/session name. */
export function menuLabel(label: string): string {
  const characters = Array.from(label.replace(/\s+/gu, " ").trim());
  return characters.length > 80 ? `${characters.slice(0, 79).join("")}…` : characters.join("");
}
