import type { ShortcutId } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";

/**
 * The saved accelerator for a shortcut, or null when it is disabled: HUD hints
 * follow Settings → Keybinds instead of a hard-coded default (UX-026).
 */
export function useShortcutAccelerator(id: ShortcutId): string | null {
  return useSettingsStore(
    (s) => s.settings?.shortcuts.find((binding) => binding.id === id && binding.enabled)?.accelerator ?? null,
  );
}
