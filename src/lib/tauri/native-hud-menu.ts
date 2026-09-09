import { getCurrentWindow } from "@tauri-apps/api/window";

import { bluey } from "./api";
import {
  MAX_HUD_MENU_ITEMS,
  MAX_HUD_MENU_POSITION,
  type HudMenuAlign,
  type HudMenuPosition,
  type HudMenuRequest,
} from "./hud-menu-types";
import { menuLabel, type MenuEntry } from "./menu-model";
import { hasTauriRuntime } from "./transport";

/** A popup-local allowlist; domain IDs, closures, URLs and React nodes never cross IPC. */
export function nativeHudMenuRequest<Action>(
  entries: readonly MenuEntry<Action>[],
  position: HudMenuPosition,
  align: HudMenuAlign,
): { request: HudMenuRequest; actions: ReadonlyMap<string, Action> } {
  if (entries.length === 0 || entries.length > MAX_HUD_MENU_ITEMS) {
    throw new Error("HUD menu item count is out of bounds");
  }
  if ([position.x, position.y].some((v) => !Number.isFinite(v) || v < 0 || v > MAX_HUD_MENU_POSITION)) {
    throw new Error("Invalid HUD menu client position");
  }
  const actions = new Map<string, Action>();
  const items = entries.map((entry, index): HudMenuRequest["items"][number] => {
    const id = `hud-${index}`;
    if (entry.kind === "separator") return { kind: "separator", id };
    const label = menuLabel(entry.label.replace(/[\p{Cc}\u2028\u2029]/gu, " ")) || "—";
    if (entry.kind === "label") return { kind: "label", id, label };
    if (!entry.disabled) actions.set(id, entry.action);
    return {
      kind: "item",
      id,
      label,
      enabled: !entry.disabled,
      checked: entry.checked ?? null,
      icon: entry.nativeIcon ?? null,
      destructive: entry.destructive ?? false,
    };
  });
  return { request: { items, position: { ...position }, align }, actions };
}

// Shared by ModeMenu and SessionMenu. Acquire before the first await; never queue stale popups.
let open = false;

/** null means unavailable/busy, otherwise the promise owns its lease through tracking + focus. */
export function openNativeHudMenu<Action>(
  entries: readonly MenuEntry<Action>[],
  trigger: HTMLButtonElement,
  align: HudMenuAlign,
): Promise<{ action: Action } | null> | null {
  if (open || !hasTauriRuntime() || !trigger.isConnected || trigger.disabled) return null;
  open = true;
  const doc = trigger.ownerDocument;
  let restoreFocus = false;
  const canRestoreFocus = () =>
    trigger.isConnected &&
    !trigger.disabled &&
    doc.hasFocus() &&
    (doc.activeElement === trigger || doc.activeElement === doc.body);
  return (async () => {
    try {
      restoreFocus = doc.hasFocus() && doc.activeElement === trigger;
      const rect = trigger.getBoundingClientRect();
      const { request, actions } = nativeHudMenuRequest(
        entries,
        { x: align === "end" ? rect.right : rect.left, y: rect.bottom },
        align,
      );
      const id = await bluey.hudMenu.popup({ request });
      // Validate against the immutable open-time action allowlist, not current React entries.
      return id !== null && actions.has(id) ? { action: actions.get(id) as Action } : null;
    } finally {
      try {
        // Restore only DOM focus that this trigger already had, before any selected action
        // can open Settings. A read-only native focus query never activates the HUD/app.
        if (restoreFocus && canRestoreFocus() && (await getCurrentWindow().isFocused())) {
          if (canRestoreFocus()) trigger.focus({ preventScroll: true });
        }
      } catch {
        // Focus restoration is best effort and must not mask the popup result/error.
      } finally {
        open = false;
      }
    }
  })();
}
