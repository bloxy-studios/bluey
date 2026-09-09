/** Display-only IPC contract; mirrored by bluey-protocols::hud_menu. Never send actions. */
export type HudMenuIcon = "manage" | "play" | "pause" | "stop" | "history";
export type HudMenuAlign = "start" | "end";
export interface HudMenuPosition {
  /** Client logical pixels from the top-left of the undecorated HUD content view. */
  x: number;
  y: number;
}
export type NativeHudMenuItem =
  | {
      kind: "item";
      id: string;
      label: string;
      enabled: boolean;
      checked: boolean | null;
      icon: HudMenuIcon | null;
      destructive: boolean;
    }
  | { kind: "label"; id: string; label: string }
  | { kind: "separator"; id: string };
export interface HudMenuRequest {
  items: NativeHudMenuItem[];
  position: HudMenuPosition;
  align: HudMenuAlign;
}
export const MAX_HUD_MENU_ITEMS = 128;
export const MAX_HUD_MENU_POSITION = 16_384;
