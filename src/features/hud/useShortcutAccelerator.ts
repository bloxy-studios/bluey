import type { ShortcutId } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";

/**
 * Keys the HUD handles itself while it has focus (useHudShortcuts), whatever the
 * global binding says: New Chat and Settings are HUD-local by default so the
 * frontmost app keeps ⌘R and ⌘, (UX-001).
 */
const HUD_LOCAL_ACCELERATORS: Partial<Record<ShortcutId, string>> = {
  new_chat: "CmdOrCtrl+KeyR",
  open_settings: "CmdOrCtrl+Comma",
};

/**
 * The accelerator to show next to a HUD action: the saved global binding when it
 * is enabled, else the HUD-local key that still works, else null (UX-026).
 */
export function useShortcutAccelerator(id: ShortcutId): string | null {
  return useSettingsStore(
    (s) =>
      s.settings?.shortcuts.find((binding) => binding.id === id && binding.enabled)?.accelerator ??
      HUD_LOCAL_ACCELERATORS[id] ??
      null,
  );
}
