import { create } from "zustand";

import { showErrorToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type BlueyError, type Settings, type SettingsPatch } from "@/lib/types";

interface SettingsStore {
  settings: Settings | null;
  lastError: BlueyError | null;
  /** From `settings.changed` events. */
  applyRemote(settings: Settings): void;
  load(): Promise<void>;
  /**
   * Persist a patch through the backend; state is updated from the result. Never throws:
   * a failure is recorded in `lastError`, shown as an error toast and resolves `null`, so
   * callers can fire-and-forget and only need the return value to confirm success.
   */
  update(patch: SettingsPatch): Promise<Settings | null>;
  reset(): Promise<void>;
}

export const useSettingsStore = create<SettingsStore>((set) => ({
  settings: null,
  lastError: null,
  applyRemote: (settings) => set({ settings }),
  load: async () => {
    try {
      set({ settings: await bluey.settings.get(), lastError: null });
    } catch (error) {
      set({ lastError: toBlueyError(error) });
    }
  },
  update: async (patch) => {
    try {
      const settings = await bluey.settings.update({ patch });
      set({ settings, lastError: null });
      return settings;
    } catch (error) {
      const failure = toBlueyError(error, "configuration");
      set({ lastError: failure });
      showErrorToast(failure);
      return null;
    }
  },
  reset: async () => {
    try {
      set({ settings: await bluey.settings.reset(), lastError: null });
    } catch (error) {
      set({ lastError: toBlueyError(error) });
    }
  },
}));
